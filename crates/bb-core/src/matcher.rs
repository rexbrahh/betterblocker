//! Core Matching Engine
//!
//! This is the hot path - every request goes through here.
//! Performance is critical: minimize allocations, use zero-copy views.

use std::collections::HashSet;

use crate::hash::hash_domain;
use crate::psl::walk_host_suffixes;
use crate::snapshot::{
    Snapshot, decode_posting_list, PatternOp, NO_PATTERN, NO_CONSTRAINT,
    read_u32_le, read_u16_le,
};
use crate::types::{
    MatchDecision, MatchResult, PartyMask, RequestContext, RuleAction, RuleFlags,
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
        
        // A2: removeparam would go here
        
        // A3: Static network filtering
        self.match_static_filters(ctx)
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

    /// Match against domain hash sets.
    fn match_domain_sets(&self, ctx: &RequestContext<'_>, candidates: &mut Vec<MatchCandidate>) {
        let allow_set = self.snapshot.domain_allow_set();
        let block_set = self.snapshot.domain_block_set();
        let rules = self.snapshot.rules();

        // Walk suffixes from most specific to least
        for suffix in walk_host_suffixes(ctx.req_host) {
            let hash = hash_domain(&suffix);

            // Check allow set
            if let Some(rule_id) = allow_set.lookup(hash) {
                let rule_id = rule_id as usize;
                if self.check_rule_options(rule_id, ctx) && self.check_domain_constraints(rule_id, ctx) {
                    candidates.push(MatchCandidate {
                        rule_id,
                        action: RuleAction::Allow,
                        is_important: false,
                        priority: 0,
                    });
                }
            }

            // Check block set
            if let Some(rule_id) = block_set.lookup(hash) {
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
        
        if constraint_off == NO_CONSTRAINT {
            return true;
        }

        let constraints = self.snapshot.domain_constraints();
        if constraint_off as usize + 4 > constraints.len() {
            return true;
        }

        let include_count = read_u16_le(constraints, constraint_off as usize) as usize;
        let exclude_count = read_u16_le(constraints, constraint_off as usize + 2) as usize;

        let site_hash = hash_domain(ctx.site_etld1);
        let mut pos = constraint_off as usize + 4;

        // Check include list
        if include_count > 0 {
            let mut found = false;
            for _ in 0..include_count {
                if pos + 8 > constraints.len() {
                    break;
                }
                let lo = read_u32_le(constraints, pos);
                let hi = read_u32_le(constraints, pos + 4);
                pos += 8;

                if lo == site_hash.lo && hi == site_hash.hi {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
            // Skip remaining include entries
            pos = constraint_off as usize + 4 + include_count * 8;
        }

        // Check exclude list
        for _ in 0..exclude_count {
            if pos + 8 > constraints.len() {
                break;
            }
            let lo = read_u32_le(constraints, pos);
            let hi = read_u32_le(constraints, pos + 4);
            pos += 8;

            if lo == site_hash.lo && hi == site_hash.hi {
                return false; // Excluded
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
        let url_lower = url.to_lowercase();
        let url_bytes = url_lower.as_bytes();
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
                        Some(s) => s.to_lowercase(),
                        None => return false,
                    };

                    match url_lower[url_pos..].find(&literal) {
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
                    if !is_at_boundary(&url_lower, url_pos) {
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
        let mut best_allow: Option<&MatchCandidate> = None;
        let mut best_block: Option<&MatchCandidate> = None;
        let mut best_redirect: Option<&MatchCandidate> = None;

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
                    if best_allow.map_or(true, |b| c.priority > b.priority) {
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

        // 1. IMPORTANT BLOCK wins (ignores exceptions)
        if let Some(c) = best_important_block {
            let list_id = rules.list_id(c.rule_id);

            if let Some(redirect) = best_redirect {
                if let Some(url) = self.get_redirect_url(redirect.rule_id) {
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

            if let Some(redirect) = best_redirect {
                if let Some(url) = self.get_redirect_url(redirect.rule_id) {
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
        let option_id = rules.option_id(rule_id);
        
        if option_id == 0xFFFF_FFFF {
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


