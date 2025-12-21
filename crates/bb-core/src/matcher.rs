//! Core Matching Engine
//!
//! This is the hot path - every request goes through here.
//! Performance is critical: minimize allocations, use zero-copy views.

use std::collections::HashSet;

use crate::hash::hash_domain;
use crate::psl::walk_host_suffixes;
use crate::snapshot::{
    Snapshot, decode_posting_list, decode_posting_list_with_count, PatternOp, NO_PATTERN, NO_CONSTRAINT,
    read_u32_le, read_u16_le,
};
use crate::types::{
    MatchDecision, MatchResult, PartyMask, RequestContext, RequestType, RuleAction, RuleFlags,
};
use crate::url::{extract_host, is_at_boundary, get_host_position, tokenize_url};

// =============================================================================
// Matcher
// =============================================================================

/// The core matching engine.
pub struct Matcher<'a> {
    snapshot: &'a Snapshot<'a>,
    trusted_sites: HashSet<String>,
}

pub struct ResponseHeader<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

pub struct ResponseMatchResult {
    pub cancel: bool,
    pub rule_id: i32,
    pub list_id: u16,
    pub csp_injections: Vec<String>,
    pub remove_headers: Vec<String>,
}

pub struct ScriptletCall {
    pub name: String,
    pub args: Vec<String>,
}

pub struct CosmeticMatchResult {
    pub css: String,
    pub enable_generic: bool,
    pub scriptlets: Vec<ScriptletCall>,
    pub procedural: Vec<String>,
}

const NO_OPTION_ID: u32 = 0xFFFF_FFFF;

impl Default for ResponseMatchResult {
    fn default() -> Self {
        Self {
            cancel: false,
            rule_id: -1,
            list_id: 0,
            csp_injections: Vec::new(),
            remove_headers: Vec::new(),
        }
    }
}

impl<'a> Matcher<'a> {
    /// Create a new matcher with the given snapshot.
    pub fn new(snapshot: &'a Snapshot<'a>) -> Self {
        Self {
            snapshot,
            trusted_sites: HashSet::new(),
        }
    }

    /// Add a site to the trusted list (bypass all blocking).
    pub fn add_trusted_site(&mut self, site: &str) {
        self.trusted_sites.insert(site.to_lowercase());
    }

    /// Remove a site from the trusted list.
    pub fn remove_trusted_site(&mut self, site: &str) {
        self.trusted_sites.remove(&site.to_lowercase());
    }

    /// Match a request and return the decision.
    pub fn match_request(&self, ctx: &RequestContext<'_>) -> MatchResult {
        // A0: Trusted site bypass
        if self.trusted_sites.contains(ctx.site_etld1) {
            return MatchResult::default();
        }

        // A1: Dynamic filtering would go here

        if let Some(result) = self.match_removeparam(ctx) {
            return result;
        }

        // A3: Static network filtering
        self.match_static_filters(ctx)
    }

    pub fn match_response_headers(
        &self,
        ctx: &RequestContext<'_>,
        headers: &[ResponseHeader<'_>],
    ) -> ResponseMatchResult {
        let mut result = ResponseMatchResult::default();

        let mut candidates = Vec::new();
        self.match_domain_sets(ctx, &mut candidates);
        self.match_token_rules(ctx, &mut candidates);

        let rules = self.snapshot.rules();
        let document_only = ctx.request_type.intersects(RequestType::DOCUMENT);

        let mut csp_injection_set: HashSet<&str> = HashSet::new();
        let mut csp_exceptions: HashSet<&str> = HashSet::new();
        let mut csp_disabled = false;

        let mut best_important_block: Option<&MatchCandidate> = None;
        let mut best_allow: Option<&MatchCandidate> = None;
        let mut best_block: Option<&MatchCandidate> = None;

        for candidate in &candidates {
            let option_id = rules.option_id(candidate.rule_id);
            if option_id == NO_OPTION_ID {
                continue;
            }

            match candidate.action {
                RuleAction::CspInject => {
                    if !document_only {
                        continue;
                    }
                    let flags = RuleFlags::from_bits_truncate(rules.flags(candidate.rule_id));
                    if let Some(spec) = self.get_csp_spec(option_id) {
                        if flags.contains(RuleFlags::CSP_EXCEPTION) {
                            if spec.is_empty() {
                                csp_disabled = true;
                            } else {
                                csp_exceptions.insert(spec);
                            }
                        } else {
                            csp_injection_set.insert(spec);
                        }
                    }
                }
                RuleAction::HeaderMatchBlock | RuleAction::HeaderMatchAllow => {
                    let spec = match self.get_header_spec(option_id) {
                        Some(spec) => spec,
                        None => continue,
                    };
                    if !header_matches(&spec, headers) {
                        continue;
                    }
                    if candidate.action == RuleAction::HeaderMatchAllow {
                        if best_allow.map_or(true, |b| candidate.priority > b.priority) {
                            best_allow = Some(candidate);
                        }
                        continue;
                    }

                    let flags = RuleFlags::from_bits_truncate(rules.flags(candidate.rule_id));
                    if flags.contains(RuleFlags::IMPORTANT) {
                        if best_important_block.map_or(true, |b| candidate.priority > b.priority) {
                            best_important_block = Some(candidate);
                        }
                    } else if best_block.map_or(true, |b| candidate.priority > b.priority) {
                        best_block = Some(candidate);
                    }
                }
                _ => {}
            }
        }

        if document_only && !csp_disabled {
            for spec in csp_injection_set {
                if !csp_exceptions.contains(spec) {
                    result.csp_injections.push(spec.to_string());
                }
            }
        }

        if document_only {
            let section = self.snapshot.responseheader_rules();
            if section.len() >= 4 {
                let mut remove_set: HashSet<&str> = HashSet::new();
                let mut exception_set: HashSet<&str> = HashSet::new();
                let count = read_u32_le(section, 0) as usize;
                for idx in 0..count {
                    let entry_offset = 4 + idx * 16;
                    if entry_offset + 16 > section.len() {
                        break;
                    }
                    let constraint_offset = read_u32_le(section, entry_offset);
                    if !self.check_domain_constraints_offset(constraint_offset, ctx) {
                        continue;
                    }
                    let name_off = read_u32_le(section, entry_offset + 4) as usize;
                    let name_len = read_u32_le(section, entry_offset + 8) as usize;
                    let flags = read_u16_le(section, entry_offset + 12);

                    let header = match self.snapshot.get_string(name_off, name_len) {
                        Some(name) => name,
                        None => continue,
                    };

                    if !is_safe_response_header(header) {
                        continue;
                    }

                    if flags & 1 != 0 {
                        exception_set.insert(header);
                    } else {
                        remove_set.insert(header);
                    }
                }

                for header in remove_set {
                    if !exception_set.contains(header) {
                        result.remove_headers.push(header.to_string());
                    }
                }
            }
        }

        if let Some(c) = best_important_block {
            result.cancel = true;
            result.rule_id = c.rule_id as i32;
            result.list_id = rules.list_id(c.rule_id);
            return result;
        }

        if best_allow.is_some() && best_block.is_some() {
            return result;
        }

        if let Some(c) = best_block {
            result.cancel = true;
            result.rule_id = c.rule_id as i32;
            result.list_id = rules.list_id(c.rule_id);
        }

        result
    }

    pub fn match_cosmetics(&self, ctx: &RequestContext<'_>) -> CosmeticMatchResult {
        let mut result = CosmeticMatchResult {
            css: String::new(),
            enable_generic: true,
            scriptlets: Vec::new(),
            procedural: Vec::new(),
        };

        let mut candidates = Vec::new();
        self.match_domain_sets(ctx, &mut candidates);
        self.match_token_rules(ctx, &mut candidates);

        let rules = self.snapshot.rules();
        let mut elemhide_disabled = false;
        let mut generichide_disabled = false;

        for candidate in &candidates {
            if candidate.action != RuleAction::Allow {
                continue;
            }
            let flags = RuleFlags::from_bits_truncate(rules.flags(candidate.rule_id));
            if flags.contains(RuleFlags::ELEMHIDE) {
                elemhide_disabled = true;
            }
            if flags.contains(RuleFlags::GENERICHIDE) {
                generichide_disabled = true;
            }
        }

        let mut specific_selectors: HashSet<&str> = HashSet::new();
        let mut generic_selectors: HashSet<&str> = HashSet::new();
        let mut exception_selectors: HashSet<&str> = HashSet::new();

        let section = self.snapshot.cosmetic_rules();
        if section.len() >= 4 {
            let count = read_u32_le(section, 0) as usize;
            for idx in 0..count {
                let entry_offset = 4 + idx * 16;
                if entry_offset + 16 > section.len() {
                    break;
                }
                let constraint_offset = read_u32_le(section, entry_offset);
                if !self.check_domain_constraints_offset(constraint_offset, ctx) {
                    continue;
                }
                let selector_off = read_u32_le(section, entry_offset + 4) as usize;
                let selector_len = read_u32_le(section, entry_offset + 8) as usize;
                let flags = read_u16_le(section, entry_offset + 12);

                let selector = match self.snapshot.get_string(selector_off, selector_len) {
                    Some(value) => value,
                    None => continue,
                };

                let is_exception = flags & 1 != 0;
                let is_generic = flags & (1 << 1) != 0;

                if is_exception {
                    exception_selectors.insert(selector);
                } else if is_generic {
                    generic_selectors.insert(selector);
                } else {
                    specific_selectors.insert(selector);
                }
            }
        }

        if !elemhide_disabled {
            let mut selectors: Vec<&str> = Vec::new();
            for selector in specific_selectors {
                if !exception_selectors.contains(selector) {
                    selectors.push(selector);
                }
            }
            if !generichide_disabled {
                for selector in generic_selectors {
                    if !exception_selectors.contains(selector) {
                        selectors.push(selector);
                    }
                }
            }

            if !selectors.is_empty() {
                result.css = format!("{}{{display:none !important;}}", selectors.join(",\n"));
            }
        }

        result.enable_generic = !generichide_disabled;

        if !elemhide_disabled {
            let mut procedural_specific: HashSet<&str> = HashSet::new();
            let mut procedural_generic: HashSet<&str> = HashSet::new();
            let mut procedural_exceptions: HashSet<&str> = HashSet::new();

            let section = self.snapshot.procedural_rules();
            if section.len() >= 4 {
                let count = read_u32_le(section, 0) as usize;
                for idx in 0..count {
                    let entry_offset = 4 + idx * 16;
                    if entry_offset + 16 > section.len() {
                        break;
                    }
                    let constraint_offset = read_u32_le(section, entry_offset);
                    if !self.check_domain_constraints_offset(constraint_offset, ctx) {
                        continue;
                    }
                    let selector_off = read_u32_le(section, entry_offset + 4) as usize;
                    let selector_len = read_u32_le(section, entry_offset + 8) as usize;
                    let flags = read_u16_le(section, entry_offset + 12);

                    let selector = match self.snapshot.get_string(selector_off, selector_len) {
                        Some(value) => value,
                        None => continue,
                    };

                    let is_exception = flags & 1 != 0;
                    let is_generic = flags & (1 << 1) != 0;

                    if is_exception {
                        procedural_exceptions.insert(selector);
                    } else if is_generic {
                        procedural_generic.insert(selector);
                    } else {
                        procedural_specific.insert(selector);
                    }
                }
            }

            let mut selectors: Vec<&str> = Vec::new();
            for selector in procedural_specific {
                if !procedural_exceptions.contains(selector) {
                    selectors.push(selector);
                }
            }
            if !generichide_disabled {
                for selector in procedural_generic {
                    if !procedural_exceptions.contains(selector) {
                        selectors.push(selector);
                    }
                }
            }

            for selector in selectors {
                result.procedural.push(selector.to_string());
            }
        }

        let section = self.snapshot.scriptlet_rules();
        if section.len() >= 4 {
            let count = read_u32_le(section, 0) as usize;
            let mut scriptlet_candidates: HashSet<&str> = HashSet::new();
            let mut scriptlet_exceptions: HashSet<&str> = HashSet::new();
            let mut scriptlet_disable_all = false;

            for idx in 0..count {
                let entry_offset = 4 + idx * 16;
                if entry_offset + 16 > section.len() {
                    break;
                }
                let constraint_offset = read_u32_le(section, entry_offset);
                if !self.check_domain_constraints_offset(constraint_offset, ctx) {
                    continue;
                }
                let scriptlet_off = read_u32_le(section, entry_offset + 4) as usize;
                let scriptlet_len = read_u32_le(section, entry_offset + 8) as usize;
                let flags = read_u16_le(section, entry_offset + 12);

                let scriptlet_raw = match self.snapshot.get_string(scriptlet_off, scriptlet_len) {
                    Some(value) => value,
                    None => continue,
                };

                let is_exception = flags & 1 != 0;
                let is_generic = flags & (1 << 1) != 0;

                if is_exception && scriptlet_raw.is_empty() {
                    scriptlet_disable_all = true;
                    continue;
                }

                if is_generic {
                    continue;
                }

                if is_exception {
                    scriptlet_exceptions.insert(scriptlet_raw);
                } else {
                    scriptlet_candidates.insert(scriptlet_raw);
                }
            }

            if !scriptlet_disable_all {
                for scriptlet_raw in scriptlet_candidates {
                    if scriptlet_exceptions.contains(scriptlet_raw) {
                        continue;
                    }
                    if let Some(call) = parse_scriptlet_call(scriptlet_raw) {
                        result.scriptlets.push(call);
                    }
                }
            }
        }

        result
    }


    /// Match against static filters.
    fn match_static_filters(&self, ctx: &RequestContext<'_>) -> MatchResult {
        let mut candidates = Vec::new();

        // Step 1: Check domain sets (host-only rules)
        self.match_domain_sets(ctx, &mut candidates);

        // Step 2: Check token-indexed URL rules
        self.match_token_rules(ctx, &mut candidates);

        // Step 3: Apply precedence logic
        self.apply_precedence(&candidates)
    }

    fn match_removeparam(&self, ctx: &RequestContext<'_>) -> Option<MatchResult> {
        let mut candidates = Vec::new();
        self.match_token_rules(ctx, &mut candidates);

        if candidates.is_empty() {
            return None;
        }

        let rules = self.snapshot.rules();
        let mut exception_ids: HashSet<u32> = HashSet::new();
        let mut remove_rules: Vec<(usize, u32)> = Vec::new();

        for candidate in candidates {
            let option_id = rules.option_id(candidate.rule_id);
            if option_id == NO_OPTION_ID {
                continue;
            }
            match candidate.action {
                RuleAction::Allow => {
                    exception_ids.insert(option_id);
                }
                RuleAction::Removeparam => {
                    remove_rules.push((candidate.rule_id, option_id));
                }
                _ => {}
            }
        }

        if remove_rules.is_empty() {
            return None;
        }

        let mut remove_keys: Vec<&str> = Vec::new();
        let mut selected_rule: Option<usize> = None;

        for (rule_id, option_id) in remove_rules {
            if exception_ids.contains(&option_id) {
                continue;
            }

            let spec = match self.get_removeparam_spec(option_id) {
                Some(spec) => spec,
                None => continue,
            };

            for key in split_removeparam_spec(spec) {
                if !remove_keys.contains(&key) {
                    remove_keys.push(key);
                }
            }

            if selected_rule.is_none() {
                selected_rule = Some(rule_id);
            }
        }

        if remove_keys.is_empty() {
            return None;
        }

        let new_url = match remove_params(ctx.url, &remove_keys) {
            Some(url) => url,
            None => return None,
        };

        let rule_id = selected_rule?;

        Some(MatchResult {
            decision: MatchDecision::Removeparam,
            rule_id: rule_id as i32,
            list_id: rules.list_id(rule_id),
            redirect_url: Some(new_url),
        })
    }

    /// Match against domain hash sets.
    fn match_domain_sets(&self, ctx: &RequestContext<'_>, candidates: &mut Vec<MatchCandidate>) {
        let allow_set = self.snapshot.domain_allow_set();
        let block_set = self.snapshot.domain_block_set();
        let postings = self.snapshot.domain_postings();
        let legacy_domain_sets = postings.is_none();
        let postings_data = postings.unwrap_or(&[]);
        let rules = self.snapshot.rules();

        // Walk suffixes from most specific to least
        for suffix in walk_host_suffixes(ctx.req_host) {
            let hash = hash_domain(&suffix);

            // Check allow set
            if let Some(value) = allow_set.lookup(hash) {
                if legacy_domain_sets {
                    let rule_id = value as usize;
                    if self.check_rule_options(rule_id, ctx) && self.check_domain_constraints(rule_id, ctx) {
                        let flags = RuleFlags::from_bits_truncate(rules.flags(rule_id));
                        candidates.push(MatchCandidate {
                            rule_id,
                            action: RuleAction::Allow,
                            is_important: flags.contains(RuleFlags::IMPORTANT),
                            priority: 0,
                        });
                    }
                } else {
                    let rule_ids = decode_posting_list_with_count(postings_data, value as usize);
                    for rule_id in rule_ids {
                        let rule_id = rule_id as usize;
                        if self.check_rule_options(rule_id, ctx) && self.check_domain_constraints(rule_id, ctx) {
                            let flags = RuleFlags::from_bits_truncate(rules.flags(rule_id));
                            candidates.push(MatchCandidate {
                                rule_id,
                                action: RuleAction::Allow,
                                is_important: flags.contains(RuleFlags::IMPORTANT),
                                priority: 0,
                            });
                        }
                    }
                }
            }

            // Check block set
            if let Some(value) = block_set.lookup(hash) {
                if legacy_domain_sets {
                    let rule_id = value as usize;
                    if self.check_rule_options(rule_id, ctx) && self.check_domain_constraints(rule_id, ctx) {
                        let flags = RuleFlags::from_bits_truncate(rules.flags(rule_id));
                        candidates.push(MatchCandidate {
                            rule_id,
                            action: RuleAction::Block,
                            is_important: flags.contains(RuleFlags::IMPORTANT),
                            priority: 0,
                        });
                    }
                } else {
                    let rule_ids = decode_posting_list_with_count(postings_data, value as usize);
                    for rule_id in rule_ids {
                        let rule_id = rule_id as usize;
                        if self.check_rule_options(rule_id, ctx) && self.check_domain_constraints(rule_id, ctx) {
                            let flags = RuleFlags::from_bits_truncate(rules.flags(rule_id));
                            candidates.push(MatchCandidate {
                                rule_id,
                                action: RuleAction::Block,
                                is_important: flags.contains(RuleFlags::IMPORTANT),
                                priority: 0,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Match against token-indexed URL pattern rules.
    fn match_token_rules(&self, ctx: &RequestContext<'_>, candidates: &mut Vec<MatchCandidate>) {
        let token_dict = self.snapshot.token_dict();
        let postings = self.snapshot.token_postings();
        let rules = self.snapshot.rules();
        let pattern_pool = self.snapshot.pattern_pool();

        // Tokenize the URL
        let token_hashes = tokenize_url(ctx.url);
        if token_hashes.is_empty() {
            return;
        }

        // Find the rarest token to minimize candidate set
        let mut best_entry = None;
        let mut best_count = usize::MAX;

        for &hash in &token_hashes {
            if let Some(entry) = token_dict.lookup(hash) {
                if entry.rule_count < best_count {
                    best_entry = Some(entry);
                    best_count = entry.rule_count;
                }
            }
        }

        let entry = match best_entry {
            Some(e) => e,
            None => return,
        };

        // Decode the posting list
        let rule_ids = decode_posting_list(postings, entry.postings_offset, entry.rule_count);

        // Verify each candidate
        for rule_id in rule_ids {
            let rule_id = rule_id as usize;

            // Quick option checks first
            if !self.check_rule_options(rule_id, ctx) {
                continue;
            }

            // Check domain constraints
            if !self.check_domain_constraints(rule_id, ctx) {
                continue;
            }

            // Pattern verification
            let pattern_id = rules.pattern_id(rule_id);
            if pattern_id != NO_PATTERN {
                if let Some(pattern) = pattern_pool.get_pattern(pattern_id as usize) {
                    let program = pattern_pool.get_program(&pattern);
                    if !self.verify_pattern(ctx.url, &pattern, program) {
                        continue;
                    }
                }
            }

            // Rule matches!
            let action = RuleAction::try_from(rules.action(rule_id)).unwrap_or(RuleAction::Block);
            let flags = RuleFlags::from_bits_truncate(rules.flags(rule_id));
            let priority = rules.priority(rule_id);

            candidates.push(MatchCandidate {
                rule_id,
                action,
                is_important: flags.contains(RuleFlags::IMPORTANT),
                priority,
            });
        }
    }

    /// Check if a rule's options match the request context.
    fn check_rule_options(&self, rule_id: usize, ctx: &RequestContext<'_>) -> bool {
        let rules = self.snapshot.rules();

        // Type mask
        let type_mask = rules.type_mask(rule_id);
        if type_mask != 0 && (type_mask & ctx.request_type.bits()) == 0 {
            return false;
        }

        // Party mask
        let party_mask = rules.party_mask(rule_id);
        if party_mask != 0 {
            let request_party = if ctx.is_third_party {
                PartyMask::THIRD_PARTY
            } else {
                PartyMask::FIRST_PARTY
            };
            if (party_mask & request_party.bits()) == 0 {
                return false;
            }
        }

        // Scheme mask
        let scheme_mask = rules.scheme_mask(rule_id);
        if scheme_mask != 0 && (scheme_mask & ctx.scheme.bits()) == 0 {
            return false;
        }

        true
    }

    /// Check domain constraints ($domain=).
    fn check_domain_constraints(&self, rule_id: usize, ctx: &RequestContext<'_>) -> bool {
        let rules = self.snapshot.rules();
        let constraint_off = rules.domain_constraint_offset(rule_id);
        self.check_domain_constraints_offset(constraint_off, ctx)
    }

    fn check_domain_constraints_offset(&self, constraint_off: u32, ctx: &RequestContext<'_>) -> bool {
        if constraint_off == NO_CONSTRAINT {
            return true;
        }

        let constraints = self.snapshot.domain_constraints();
        let offset = constraint_off as usize;
        if offset + 4 > constraints.len() {
            return true;
        }

        let include_count = read_u16_le(constraints, offset) as usize;
        let exclude_count = read_u16_le(constraints, offset + 2) as usize;
        let include_start = offset + 4;
        let include_end = include_start + include_count * 8;
        let exclude_end = include_end + exclude_count * 8;
        if exclude_end > constraints.len() {
            return true;
        }

        let include_slice = &constraints[include_start..include_end];
        let exclude_slice = &constraints[include_end..exclude_end];

        let list_contains = |list: &[u8], lo: u32, hi: u32| -> bool {
            let mut pos = 0;
            while pos + 8 <= list.len() {
                let entry_lo = read_u32_le(list, pos);
                let entry_hi = read_u32_le(list, pos + 4);
                if entry_lo == lo && entry_hi == hi {
                    return true;
                }
                pos += 8;
            }
            false
        };

        if include_count > 0 {
            let mut matched = false;
            for suffix in walk_host_suffixes(ctx.site_host) {
                let hash = hash_domain(&suffix);
                if list_contains(include_slice, hash.lo, hash.hi) {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return false;
            }
        }

        if exclude_count > 0 {
            for suffix in walk_host_suffixes(ctx.site_host) {
                let hash = hash_domain(&suffix);
                if list_contains(exclude_slice, hash.lo, hash.hi) {
                    return false;
                }
            }
        }

        true
    }

    /// Verify a URL against a compiled pattern program.
    fn verify_pattern(
        &self,
        url: &str,
        pattern: &crate::snapshot::PatternEntry,
        program: &[u8],
    ) -> bool {
        let url_bytes = url.as_bytes();
        let mut url_pos: usize = 0;
        let mut prog_pos: usize = 0;

        while prog_pos < program.len() {
            let op = match PatternOp::try_from(program[prog_pos]) {
                Ok(op) => op,
                Err(_) => return false,
            };
            prog_pos += 1;

            match op {
                PatternOp::FindLit => {
                    if prog_pos + 6 > program.len() {
                        return false;
                    }
                    let str_off = read_u32_le(program, prog_pos) as usize;
                    let str_len = read_u16_le(program, prog_pos + 4) as usize;
                    prog_pos += 6;

                    let literal = match self.snapshot.get_string(str_off, str_len) {
                        Some(s) => s,
                        None => return false,
                    };

                    match find_case_insensitive(&url_bytes[url_pos..], literal.as_bytes()) {
                        Some(pos) => url_pos += pos + literal.len(),
                        None => return false,
                    }
                }

                PatternOp::AssertStart => {
                    if url_pos != 0 {
                        return false;
                    }
                }

                PatternOp::AssertEnd => {
                    if url_pos != url_bytes.len() {
                        return false;
                    }
                }

                PatternOp::AssertBoundary => {
                    if !is_at_boundary(url, url_pos) {
                        return false;
                    }
                }

                PatternOp::SkipAny => {
                    // Wildcard - continue
                }

                PatternOp::HostAnchor => {
                    // Verify match is within hostname portion
                    let (_host_start, host_end) = match get_host_position(url) {
                        Some(pos) => pos,
                        None => return false,
                    };

                    if pattern.host_hash_lo != 0 || pattern.host_hash_hi != 0 {
                        let req_host = match extract_host(url) {
                            Some(h) => h,
                            None => return false,
                        };

                        let mut host_matches = false;
                        for suffix in walk_host_suffixes(req_host) {
                            let suffix_hash = hash_domain(&suffix);
                            if suffix_hash.lo == pattern.host_hash_lo
                                && suffix_hash.hi == pattern.host_hash_hi
                            {
                                host_matches = true;
                                break;
                            }
                        }

                        if !host_matches {
                            return false;
                        }
                    }

                    if url_pos > host_end {
                        return false;
                    }
                }

                PatternOp::Done => {
                    return true;
                }
            }
        }

        true
    }

    /// Apply precedence rules to determine final decision.
    fn apply_precedence(&self, candidates: &[MatchCandidate]) -> MatchResult {
        if candidates.is_empty() {
            return MatchResult::default();
        }

        let rules = self.snapshot.rules();

        let mut best_important_block: Option<&MatchCandidate> = None;
        let mut best_important_allow: Option<&MatchCandidate> = None;
        let mut best_allow: Option<&MatchCandidate> = None;
        let mut best_block: Option<&MatchCandidate> = None;
        let mut best_redirect: Option<&MatchCandidate> = None;
        let mut redirect_exceptions: HashSet<u32> = HashSet::new();

        for c in candidates {
            match c.action {
                RuleAction::Block => {
                    if c.is_important {
                        if best_important_block.map_or(true, |b| c.priority > b.priority) {
                            best_important_block = Some(c);
                        }
                    } else {
                        if best_block.map_or(true, |b| c.priority > b.priority) {
                            best_block = Some(c);
                        }
                    }
                }
                RuleAction::Allow => {
                    let flags = RuleFlags::from_bits_truncate(rules.flags(c.rule_id));
                    if flags.contains(RuleFlags::REDIRECT_RULE_EXCEPTION) {
                        let option_id = rules.option_id(c.rule_id);
                        if option_id != NO_OPTION_ID {
                            redirect_exceptions.insert(option_id);
                        }
                        continue;
                    }
                    if flags.contains(RuleFlags::ELEMHIDE) || flags.contains(RuleFlags::GENERICHIDE) {
                        continue;
                    }
                    if c.is_important {
                        if best_important_allow.map_or(true, |b| c.priority > b.priority) {
                            best_important_allow = Some(c);
                        }
                    } else if best_allow.map_or(true, |b| c.priority > b.priority) {
                        best_allow = Some(c);
                    }
                }
                RuleAction::RedirectDirective => {
                    if best_redirect.map_or(true, |b| c.priority > b.priority) {
                        best_redirect = Some(c);
                    }
                }
                _ => {}
            }
        }

        // 1. IMPORTANT ALLOW beats everything (including important block)
        if let Some(c) = best_important_allow {
            return MatchResult {
                decision: MatchDecision::Allow,
                rule_id: c.rule_id as i32,
                list_id: rules.list_id(c.rule_id),
                redirect_url: None,
            };
        }

        // 2. IMPORTANT BLOCK wins over regular exceptions
        if let Some(c) = best_important_block {
            let list_id = rules.list_id(c.rule_id);

            if let Some(url) = self.get_redirect_url_by_option(rules.option_id(c.rule_id)) {
                return MatchResult {
                    decision: MatchDecision::Redirect,
                    rule_id: c.rule_id as i32,
                    list_id,
                    redirect_url: Some(url),
                };
            }

            if let Some(redirect) = best_redirect {
                let option_id = rules.option_id(redirect.rule_id);
                if option_id != NO_OPTION_ID && redirect_exceptions.contains(&option_id) {
                } else if let Some(url) = self.get_redirect_url(redirect.rule_id) {
                    return MatchResult {
                        decision: MatchDecision::Redirect,
                        rule_id: c.rule_id as i32,
                        list_id,
                        redirect_url: Some(url),
                    };
                }
            }

            return MatchResult {
                decision: MatchDecision::Block,
                rule_id: c.rule_id as i32,
                list_id,
                redirect_url: None,
            };
        }

        // 2. ALLOW exception overrides normal block
        if best_allow.is_some() && best_block.is_some() {
            let c = best_allow.unwrap();
            return MatchResult {
                decision: MatchDecision::Allow,
                rule_id: c.rule_id as i32,
                list_id: rules.list_id(c.rule_id),
                redirect_url: None,
            };
        }

        // 3. Normal BLOCK (with possible redirect)
        if let Some(c) = best_block {
            let list_id = rules.list_id(c.rule_id);

            if let Some(url) = self.get_redirect_url_by_option(rules.option_id(c.rule_id)) {
                return MatchResult {
                    decision: MatchDecision::Redirect,
                    rule_id: c.rule_id as i32,
                    list_id,
                    redirect_url: Some(url),
                };
            }

            if let Some(redirect) = best_redirect {
                let option_id = rules.option_id(redirect.rule_id);
                if option_id != NO_OPTION_ID && redirect_exceptions.contains(&option_id) {
                } else if let Some(url) = self.get_redirect_url(redirect.rule_id) {
                    return MatchResult {
                        decision: MatchDecision::Redirect,
                        rule_id: c.rule_id as i32,
                        list_id,
                        redirect_url: Some(url),
                    };
                }
            }

            return MatchResult {
                decision: MatchDecision::Block,
                rule_id: c.rule_id as i32,
                list_id,
                redirect_url: None,
            };
        }

        // 4. ALLOW (explicit or default)
        if let Some(c) = best_allow {
            return MatchResult {
                decision: MatchDecision::Allow,
                rule_id: c.rule_id as i32,
                list_id: rules.list_id(c.rule_id),
                redirect_url: None,
            };
        }

        MatchResult::default()
    }

    /// Get the redirect URL for a redirect directive.
    fn get_redirect_url(&self, rule_id: usize) -> Option<String> {
        let rules = self.snapshot.rules();
        self.get_redirect_url_by_option(rules.option_id(rule_id))
    }

    fn get_redirect_url_by_option(&self, option_id: u32) -> Option<String> {
        if option_id == NO_OPTION_ID {
            return None;
        }

        // Look up in redirect resources section
        let section = self.snapshot.get_section(crate::snapshot::SectionId::RedirectResources)?;
        if section.len() < 4 {
            return None;
        }

        let resource_count = read_u32_le(section, 0) as usize;
        if option_id as usize >= resource_count {
            return None;
        }

        let entry_offset = 4 + option_id as usize * 20;
        if entry_offset + 16 > section.len() {
            return None;
        }

        let path_str_off = read_u32_le(section, entry_offset + 8) as usize;
        let path_str_len = read_u32_le(section, entry_offset + 12) as usize;

        self.snapshot.get_string(path_str_off, path_str_len).map(|s| s.to_string())
    }

    fn get_removeparam_spec(&self, option_id: u32) -> Option<&str> {
        if option_id == NO_OPTION_ID {
            return None;
        }

        let section = self.snapshot.removeparam_specs();
        if section.len() < 4 {
            return None;
        }

        let spec_count = read_u32_le(section, 0) as usize;
        if option_id as usize >= spec_count {
            return None;
        }

        let entry_offset = 4 + option_id as usize * 12;
        if entry_offset + 8 > section.len() {
            return None;
        }

        let param_off = read_u32_le(section, entry_offset) as usize;
        let param_len = read_u32_le(section, entry_offset + 4) as usize;

        self.snapshot.get_string(param_off, param_len)
    }

    fn get_csp_spec(&self, option_id: u32) -> Option<&str> {
        if option_id == NO_OPTION_ID {
            return None;
        }

        let section = self.snapshot.csp_specs();
        if section.len() < 4 {
            return None;
        }

        let spec_count = read_u32_le(section, 0) as usize;
        if option_id as usize >= spec_count {
            return None;
        }

        let entry_offset = 4 + option_id as usize * 12;
        if entry_offset + 8 > section.len() {
            return None;
        }

        let spec_off = read_u32_le(section, entry_offset) as usize;
        let spec_len = read_u32_le(section, entry_offset + 4) as usize;

        self.snapshot.get_string(spec_off, spec_len)
    }

    fn get_header_spec(&self, option_id: u32) -> Option<HeaderSpecRef<'a>> {
        if option_id == NO_OPTION_ID {
            return None;
        }

        let section = self.snapshot.header_specs();
        if section.len() < 4 {
            return None;
        }

        let spec_count = read_u32_le(section, 0) as usize;
        if option_id as usize >= spec_count {
            return None;
        }

        let entry_offset = 4 + option_id as usize * 20;
        if entry_offset + 20 > section.len() {
            return None;
        }

        let name_off = read_u32_le(section, entry_offset) as usize;
        let name_len = read_u32_le(section, entry_offset + 4) as usize;
        let value_off = read_u32_le(section, entry_offset + 8) as usize;
        let value_len = read_u32_le(section, entry_offset + 12) as usize;
        let flags = read_u32_le(section, entry_offset + 16);

        let name = self.snapshot.get_string(name_off, name_len)?;
        let value = if value_len > 0 {
            self.snapshot.get_string(value_off, value_len)
        } else {
            None
        };

        Some(HeaderSpecRef {
            name,
            value,
            negate: flags & 1 != 0,
        })
    }
}

// =============================================================================
// Match Candidate
// =============================================================================

#[derive(Debug)]
struct MatchCandidate {
    rule_id: usize,
    action: RuleAction,
    is_important: bool,
    priority: i16,
}

struct HeaderSpecRef<'a> {
    name: &'a str,
    value: Option<&'a str>,
    negate: bool,
}

fn find_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }

    let last = haystack.len() - needle.len();
    for i in 0..=last {
        if haystack[i..i + needle.len()].eq_ignore_ascii_case(needle) {
            return Some(i);
        }
    }

    None
}

fn header_matches(spec: &HeaderSpecRef<'_>, headers: &[ResponseHeader<'_>]) -> bool {
    let mut found = false;
    let mut any_value_match = false;

    for header in headers {
        if !header.name.eq_ignore_ascii_case(spec.name) {
            continue;
        }
        found = true;

        if let Some(value) = spec.value {
            if find_case_insensitive(header.value.as_bytes(), value.as_bytes()).is_some() {
                any_value_match = true;
            }
        }
    }

    match spec.value {
        None => {
            if spec.negate {
                !found
            } else {
                found
            }
        }
        Some(_) => {
            if spec.negate {
                found && !any_value_match
            } else {
                any_value_match
            }
        }
    }
}

fn parse_scriptlet_call(raw: &str) -> Option<ScriptletCall> {
    let mut parts = raw.split(',').map(|part| part.trim()).filter(|part| !part.is_empty());
    let name = parts.next()?;
    let args = parts.map(|part| part.to_string()).collect();
    Some(ScriptletCall {
        name: name.to_string(),
        args,
    })
}

fn is_safe_response_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("location")
        || name.eq_ignore_ascii_case("refresh")
        || name.eq_ignore_ascii_case("report-to")
        || name.eq_ignore_ascii_case("set-cookie")
}

fn split_removeparam_spec(spec: &str) -> Vec<&str> {
    spec.split(|ch| ch == '|' || ch == ',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect()
}

fn remove_params(url: &str, remove_keys: &[&str]) -> Option<String> {
    let query_start = url.find('?')?;
    let fragment_start = url[query_start + 1..].find('#').map(|idx| idx + query_start + 1);

    let base = &url[..query_start];
    let query_end = fragment_start.unwrap_or_else(|| url.len());
    let query = &url[query_start + 1..query_end];
    let fragment = fragment_start.map(|idx| &url[idx..]).unwrap_or("");

    if query.is_empty() {
        return None;
    }

    let mut kept = Vec::new();
    let mut removed = false;

    for part in query.split('&') {
        if part.is_empty() {
            continue;
        }
        let name = match part.find('=') {
            Some(idx) => &part[..idx],
            None => part,
        };
        if remove_keys.contains(&name) {
            removed = true;
            continue;
        }
        kept.push(part);
    }

    if !removed {
        return None;
    }

    let mut out = String::with_capacity(url.len());
    out.push_str(base);
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    out.push_str(fragment);

    Some(out)
}

