use std::net::IpAddr;

use bb_core::hash::{hash_domain, Hash64};
use bb_core::types::{PartyMask, RequestType, RuleAction, RuleFlags, SchemeMask};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainConstraint {
    pub include: Vec<Hash64>,
    pub exclude: Vec<Hash64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeaderSpec {
    pub name: String,
    pub value: Option<String>,
    pub negate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CosmeticRule {
    pub selector: String,
    pub is_exception: bool,
    pub is_generic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScriptletRule {
    pub scriptlet: String,
    pub is_exception: bool,
    pub is_generic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProceduralRule {
    pub selector: String,
    pub is_exception: bool,
    pub is_generic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResponseHeaderRule {
    pub header: String,
    pub is_exception: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRule {
    pub action: RuleAction,
    pub flags: RuleFlags,
    pub domain: String,
    pub pattern: Option<String>,
    pub anchor_type: AnchorType,
    pub list_id: u16,
    pub type_mask: RequestType,
    pub party_mask: PartyMask,
    pub scheme_mask: SchemeMask,
    pub domain_constraints: Option<DomainConstraint>,
    pub redirect: Option<String>,
    pub removeparam: Option<String>,
    pub csp: Option<String>,
    pub header: Option<HeaderSpec>,
    pub cosmetic: Option<CosmeticRule>,
    pub procedural: Option<ProceduralRule>,
    pub scriptlet: Option<ScriptletRule>,
    pub responseheader: Option<ResponseHeaderRule>,
    pub is_badfilter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnchorType {
    #[default]
    None,
    Left,
    Hostname,
}

pub fn parse_filter_list(text: &str) -> Vec<CompiledRule> {
    let mut rules = Vec::new();

    for raw_line in text.lines() {
        let mut line = raw_line.trim();
        if line.is_empty() || is_comment_line(line) {
            continue;
        }

        if let Some(rule) = parse_responseheader_line(line) {
            rules.push(rule);
            continue;
        }

        if let Some(rule) = parse_scriptlet_line(line) {
            rules.push(rule);
            continue;
        }

        if let Some(rule) = parse_procedural_line(line) {
            rules.push(rule);
            continue;
        }

        if let Some(rule) = parse_cosmetic_line(line) {
            rules.push(rule);
            continue;
        }

        if line.contains("##") || line.contains("#@#") || line.contains("#?#") {
            continue;
        }

        let mut action = RuleAction::Block;
        if let Some(rest) = line.strip_prefix("@@") {
            action = RuleAction::Allow;
            line = rest.trim_start();
        }

        let (pattern_part, options_text) = split_rule_options(line);
        let mut options = match options_text {
            Some(options_text) => match parse_options(options_text) {
                Some(options) => options,
                None => continue,
            },
            None => ParsedOptions::default(),
        };

        let pattern_str = pattern_part.trim();
        let is_badfilter = options.is_badfilter;
        let removeparam = options.removeparam.clone();
        let csp = options.csp.clone();
        let header = options.header.clone();

        if csp.is_some() {
            if action == RuleAction::Allow {
                options.flags |= RuleFlags::CSP_EXCEPTION;
            }
            action = RuleAction::CspInject;
        } else if header.is_some() {
            action = if action == RuleAction::Allow {
                RuleAction::HeaderMatchAllow
            } else {
                RuleAction::HeaderMatchBlock
            };
        } else if removeparam.is_some() && action == RuleAction::Block {
            action = RuleAction::Removeparam;
        }

        let cosmetic_override = options.flags.intersects(RuleFlags::ELEMHIDE | RuleFlags::GENERICHIDE);
        if cosmetic_override {
            if action != RuleAction::Allow
                || removeparam.is_some()
                || csp.is_some()
                || header.is_some()
                || options.redirect.is_some()
            {
                continue;
            }
        }

        if options.removeparam.is_none() && options.csp.is_none() && options.header.is_none() {
            if let Some(domain) = parse_host_anchor_rule(pattern_str) {
                let (final_action, final_flags, redirect) = finalize_rule(action, &options);
                rules.push(CompiledRule {
                    action: final_action,
                    flags: final_flags,
                    domain,
                    pattern: None,
                    anchor_type: AnchorType::Hostname,
                    list_id: 0,
                    type_mask: options.type_mask,
                    party_mask: options.party_mask,
                    scheme_mask: options.scheme_mask,
                    domain_constraints: options.domain_constraints.clone(),
                    redirect,
                    removeparam: removeparam.clone(),
                    csp: csp.clone(),
                    header: header.clone(),
                    cosmetic: None,
                    procedural: None,
                    scriptlet: None,
                    responseheader: None,
                    is_badfilter,
                });
                continue;
            }

            if let Some(domain) = parse_hosts_file_domain(pattern_str) {
                let (final_action, final_flags, redirect) = finalize_rule(action, &options);
                rules.push(CompiledRule {
                    action: final_action,
                    flags: final_flags,
                    domain,
                    pattern: None,
                    anchor_type: AnchorType::Hostname,
                    list_id: 0,
                    type_mask: options.type_mask,
                    party_mask: options.party_mask,
                    scheme_mask: options.scheme_mask,
                    domain_constraints: options.domain_constraints.clone(),
                    redirect,
                    removeparam: removeparam.clone(),
                    csp: csp.clone(),
                    header: header.clone(),
                    cosmetic: None,
                    procedural: None,
                    scriptlet: None,
                    responseheader: None,
                    is_badfilter,
                });
                continue;
            }
        }

        if let Some(parsed) = parse_pattern_rule(pattern_str) {
            let (final_action, final_flags, redirect) = finalize_rule(action, &options);
            rules.push(CompiledRule {
                action: final_action,
                flags: final_flags,
                domain: parsed.domain,
                pattern: Some(parsed.pattern),
                anchor_type: parsed.anchor_type,
                list_id: 0,
                type_mask: options.type_mask,
                party_mask: options.party_mask,
                scheme_mask: options.scheme_mask,
                domain_constraints: options.domain_constraints,
                redirect,
                removeparam,
                csp,
                header,
                cosmetic: None,
                procedural: None,
                scriptlet: None,
                responseheader: None,
                is_badfilter,
            });
        }
    }

    rules
}

fn finalize_rule(action: RuleAction, options: &ParsedOptions) -> (RuleAction, RuleFlags, Option<String>) {
    let mut final_action = action;
    let mut final_flags = options.flags;
    let mut redirect = options.redirect.clone();

    if matches!(
        action,
        RuleAction::Removeparam | RuleAction::CspInject | RuleAction::HeaderMatchBlock | RuleAction::HeaderMatchAllow
    ) {
        return (final_action, final_flags, None);
    }

    if redirect.is_some() {
        if options.redirect_is_rule {
            if action == RuleAction::Allow {
                final_flags |= RuleFlags::REDIRECT_RULE_EXCEPTION;
            } else {
                final_action = RuleAction::RedirectDirective;
            }
        } else if action == RuleAction::Allow {
            redirect = None;
        } else {
            final_flags |= RuleFlags::FROM_REDIRECT_EQ;
        }
    }

    (final_action, final_flags, redirect)
}

#[derive(Clone)]
struct ParsedOptions {
    flags: RuleFlags,
    type_mask: RequestType,
    party_mask: PartyMask,
    scheme_mask: SchemeMask,
    domain_constraints: Option<DomainConstraint>,
    redirect: Option<String>,
    redirect_is_rule: bool,
    removeparam: Option<String>,
    csp: Option<String>,
    header: Option<HeaderSpec>,
    is_badfilter: bool,
}

impl Default for ParsedOptions {
    fn default() -> Self {
        Self {
            flags: RuleFlags::empty(),
            type_mask: RequestType::from_bits_truncate(0),
            party_mask: PartyMask::from_bits_truncate(0),
            scheme_mask: SchemeMask::from_bits_truncate(0),
            domain_constraints: None,
            redirect: None,
            redirect_is_rule: false,
            removeparam: None,
            csp: None,
            header: None,
            is_badfilter: false,
        }
    }
}

fn split_rule_options(line: &str) -> (&str, Option<&str>) {
    match line.find('$') {
        Some(pos) => (&line[..pos], Some(&line[pos + 1..])),
        None => (line, None),
    }
}

fn parse_options(text: &str) -> Option<ParsedOptions> {
    let mut flags = RuleFlags::empty();
    let mut type_include = 0u32;
    let mut type_exclude = 0u32;
    let mut party_include = 0u8;
    let mut party_exclude = 0u8;
    let mut scheme_include = 0u8;
    let mut scheme_exclude = 0u8;
    let mut domain_constraints: Option<DomainConstraint> = None;
    let mut redirect: Option<String> = None;
    let mut redirect_is_rule = false;
    let mut removeparam: Option<String> = None;
    let mut csp: Option<String> = None;
    let mut header: Option<HeaderSpec> = None;
    let mut is_badfilter = false;

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(ParsedOptions::default());
    }

    for raw in trimmed.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        let raw_lower = raw.to_ascii_lowercase();
        let raw_lower = raw_lower.as_str();

        if raw_lower == "important" {
            flags |= RuleFlags::IMPORTANT;
            continue;
        }

        if raw_lower == "match-case" || raw_lower == "match_case" {
            flags |= RuleFlags::MATCH_CASE;
            continue;
        }

        if raw_lower == "badfilter" {
            is_badfilter = true;
            continue;
        }

        if raw_lower == "elemhide" {
            flags |= RuleFlags::ELEMHIDE;
            continue;
        }

        if raw_lower == "generichide" {
            flags |= RuleFlags::GENERICHIDE;
            continue;
        }

        if let Some(domain_value) = raw_lower.strip_prefix("domain=") {
            let parsed = parse_domain_option(domain_value)?;
            domain_constraints = Some(merge_constraints(domain_constraints, parsed));
            continue;
        }

        if let Some(redirect_value) = raw_lower.strip_prefix("redirect=") {
            if !redirect_value.is_empty() {
                redirect = Some(redirect_value.to_string());
                redirect_is_rule = false;
            }
            continue;
        }

        if let Some(redirect_value) = raw_lower.strip_prefix("redirect-rule=") {
            if !redirect_value.is_empty() {
                redirect = Some(redirect_value.to_string());
                redirect_is_rule = true;
            }
            continue;
        }

        if raw_lower == "csp" {
            if csp.is_some() || header.is_some() || removeparam.is_some() {
                return None;
            }
            csp = Some(String::new());
            continue;
        }

        if let Some(_csp_value) = raw_lower.strip_prefix("csp=") {
            if csp.is_some() || header.is_some() || removeparam.is_some() {
                return None;
            }
            csp = Some(raw[4..].trim().to_string());
            continue;
        }

        if let Some(_header_value) = raw_lower.strip_prefix("header=") {
            if csp.is_some() || header.is_some() || removeparam.is_some() {
                return None;
            }
            let spec = parse_header_option(raw[7..].trim())?;
            header = Some(spec);
            continue;
        }

        if let Some(removeparam_value) = raw_lower.strip_prefix("removeparam=") {
            if removeparam_value.is_empty() || csp.is_some() || header.is_some() {
                return None;
            }
            removeparam = Some(removeparam_value.to_string());
            continue;
        }

        let (negated, name) = match raw_lower.strip_prefix('~') {
            Some(rest) => (true, rest),
            None => (false, raw_lower),
        };

        if name.is_empty() || name.contains('=') {
            return None;
        }

        if let Some(mask) = request_type_mask(name) {
            if negated {
                type_exclude |= mask;
            } else {
                type_include |= mask;
            }
            continue;
        }

        if let Some(mask) = party_mask(name) {
            if negated {
                party_exclude |= mask;
            } else {
                party_include |= mask;
            }
            continue;
        }

        if let Some(mask) = scheme_mask(name) {
            if negated {
                scheme_exclude |= mask;
            } else {
                scheme_include |= mask;
            }
            continue;
        }

        return None;
    }

    let type_bits = finalize_mask_u32(type_include, type_exclude, RequestType::ALL.bits())?;
    let party_bits = finalize_mask_u8(party_include, party_exclude, PartyMask::ALL.bits())?;
    let scheme_bits = finalize_mask_u8(scheme_include, scheme_exclude, SchemeMask::ALL.bits())?;

    Some(ParsedOptions {
        flags,
        type_mask: RequestType::from_bits_truncate(type_bits),
        party_mask: PartyMask::from_bits_truncate(party_bits),
        scheme_mask: SchemeMask::from_bits_truncate(scheme_bits),
        domain_constraints,
        redirect,
        redirect_is_rule,
        removeparam,
        csp,
        header,
        is_badfilter,
    })
}

fn merge_constraints(existing: Option<DomainConstraint>, incoming: DomainConstraint) -> DomainConstraint {
    match existing {
        Some(mut current) => {
            current.include.extend(incoming.include);
            current.exclude.extend(incoming.exclude);
            current
        }
        None => incoming,
    }
}

fn parse_domain_option(value: &str) -> Option<DomainConstraint> {
    let mut include = Vec::new();
    let mut exclude = Vec::new();

    for raw in value.split('|') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        let (is_exclude, domain_raw) = match raw.strip_prefix('~') {
            Some(rest) => (true, rest),
            None => (false, raw),
        };

        let domain = normalize_domain(domain_raw)?;
        let hash = hash_domain(&domain);

        if is_exclude {
            exclude.push(hash);
        } else {
            include.push(hash);
        }
    }

    if include.is_empty() && exclude.is_empty() {
        return None;
    }

    Some(DomainConstraint { include, exclude })
}

fn parse_cosmetic_domains(value: &str) -> Option<DomainConstraint> {
    let mut include = Vec::new();
    let mut exclude = Vec::new();

    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    for raw in trimmed.split(',') {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        let (is_exclude, domain_raw) = match raw.strip_prefix('~') {
            Some(rest) => (true, rest.trim()),
            None => (false, raw),
        };

        let domain = normalize_domain(domain_raw)?;
        let hash = hash_domain(&domain);

        if is_exclude {
            exclude.push(hash);
        } else {
            include.push(hash);
        }
    }

    if include.is_empty() && exclude.is_empty() {
        None
    } else {
        Some(DomainConstraint { include, exclude })
    }
}

fn parse_header_option(raw: &str) -> Option<HeaderSpec> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let mut parts = raw.splitn(2, ':');
    let name = parts.next()?.trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }

    let mut negate = false;
    let mut value = parts.next().map(|v| v.trim().to_string());
    if let Some(current) = value.as_mut() {
        if let Some(stripped) = current.strip_prefix('~') {
            negate = true;
            *current = stripped.trim().to_string();
        }

        if current.starts_with('/') && current.ends_with('/') && current.len() > 1 {
            return None;
        }

        if current.is_empty() {
            value = None;
        }
    }

    Some(HeaderSpec {
        name: name.to_ascii_lowercase(),
        value,
        negate,
    })
}

fn finalize_mask_u32(include: u32, exclude: u32, all: u32) -> Option<u32> {
    let include = include & all;
    let exclude = exclude & all;
    let mut mask = if include != 0 { include & !exclude } else { all & !exclude };
    if mask == 0 {
        return None;
    }
    if mask == all {
        mask = 0;
    }
    Some(mask)
}

fn finalize_mask_u8(include: u8, exclude: u8, all: u8) -> Option<u8> {
    let include = include & all;
    let exclude = exclude & all;
    let mut mask = if include != 0 { include & !exclude } else { all & !exclude };
    if mask == 0 {
        return None;
    }
    if mask == all {
        mask = 0;
    }
    Some(mask)
}

fn request_type_mask(name: &str) -> Option<u32> {
    match name {
        "script" => Some(RequestType::SCRIPT.bits()),
        "image" => Some(RequestType::IMAGE.bits()),
        "stylesheet" => Some(RequestType::STYLESHEET.bits()),
        "object" => Some(RequestType::OBJECT.bits()),
        "subdocument" => Some(RequestType::SUBDOCUMENT.bits()),
        "document" | "main_frame" => Some(RequestType::MAIN_FRAME.bits()),
        "xmlhttprequest" | "xhr" => Some(RequestType::XMLHTTPREQUEST.bits()),
        "media" => Some(RequestType::MEDIA.bits()),
        "font" => Some(RequestType::FONT.bits()),
        "ping" => Some(RequestType::PING.bits()),
        "websocket" => Some(RequestType::WEBSOCKET.bits()),
        "beacon" => Some(RequestType::BEACON.bits()),
        "fetch" => Some(RequestType::FETCH.bits()),
        "csp" | "csp_report" => Some(RequestType::CSP_REPORT.bits()),
        "other" => Some(RequestType::OTHER.bits()),
        _ => None,
    }
}

fn party_mask(name: &str) -> Option<u8> {
    match name {
        "third-party" | "thirdparty" | "3p" => Some(PartyMask::THIRD_PARTY.bits()),
        "first-party" | "firstparty" | "1p" => Some(PartyMask::FIRST_PARTY.bits()),
        _ => None,
    }
}

fn scheme_mask(name: &str) -> Option<u8> {
    match name {
        "http" => Some(SchemeMask::HTTP.bits()),
        "https" => Some(SchemeMask::HTTPS.bits()),
        "ws" => Some(SchemeMask::WS.bits()),
        "wss" => Some(SchemeMask::WSS.bits()),
        "data" => Some(SchemeMask::DATA.bits()),
        "ftp" => Some(SchemeMask::FTP.bits()),
        _ => None,
    }
}

fn is_cosmetic_marker(line: &str) -> bool {
    line.starts_with("##")
        || line.starts_with("#@#")
        || line.starts_with("#?#")
        || line.starts_with("#@?#")
        || line.starts_with("##+js(")
        || line.starts_with("#@#+js(")
}

fn is_comment_line(line: &str) -> bool {
    if line.starts_with('!') || line.starts_with('[') {
        return true;
    }
    line.starts_with('#') && !is_cosmetic_marker(line)
}

fn parse_host_anchor_rule(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("||") {
        return None;
    }

    let mut rest = &line[2..];
    if rest.starts_with('.') {
        rest = &rest[1..];
    }

    let mut end = rest.len();
    for (i, ch) in rest.char_indices() {
        if ch == '^' || ch == '|' {
            end = i;
            break;
        }
        if ch == '/' || ch == '?' || ch == '#' || ch == ':' {
            return None;
        }
    }

    let host = &rest[..end];
    normalize_domain(host)
}

fn parse_hosts_file_domain(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    let second = parts.next()?;

    if first.parse::<IpAddr>().is_ok() {
        return normalize_domain(second);
    }

    None
}

fn normalize_domain(host: &str) -> Option<String> {
    let trimmed = host.trim().trim_matches('.');
    if trimmed.is_empty() {
        return None;
    }

    if !trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return None;
    }

    Some(trimmed.to_ascii_lowercase())
}

fn make_special_rule() -> CompiledRule {
    CompiledRule {
        action: RuleAction::ResponseCancel,
        flags: RuleFlags::empty(),
        domain: String::new(),
        pattern: None,
        anchor_type: AnchorType::None,
        list_id: 0,
        type_mask: RequestType::from_bits_truncate(0),
        party_mask: PartyMask::from_bits_truncate(0),
        scheme_mask: SchemeMask::from_bits_truncate(0),
        domain_constraints: None,
        redirect: None,
        removeparam: None,
        csp: None,
        header: None,
        cosmetic: None,
        procedural: None,
        scriptlet: None,
        responseheader: None,
        is_badfilter: false,
    }
}

fn parse_responseheader_line(line: &str) -> Option<CompiledRule> {
    let exception_marker = "#@#^responseheader(";
    let normal_marker = "##^responseheader(";

    let (marker, is_exception, marker_pos) = if let Some(pos) = line.find(exception_marker) {
        (exception_marker, true, pos)
    } else if let Some(pos) = line.find(normal_marker) {
        (normal_marker, false, pos)
    } else {
        return None;
    };

    let domain_part = line[..marker_pos].trim();
    let start = marker_pos + marker.len();
    let end = line.rfind(')')?;
    if end <= start {
        return None;
    }

    let header_raw = line[start..end].trim();
    if header_raw.is_empty() {
        return None;
    }

    if !header_raw
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }

    let mut rule = make_special_rule();
    rule.domain_constraints = parse_cosmetic_domains(domain_part);
    rule.responseheader = Some(ResponseHeaderRule {
        header: header_raw.to_ascii_lowercase(),
        is_exception,
    });
    Some(rule)
}

fn parse_scriptlet_line(line: &str) -> Option<CompiledRule> {
    let exception_marker = "#@#+js(";
    let normal_marker = "##+js(";

    let (marker, is_exception, marker_pos) = if let Some(pos) = line.find(exception_marker) {
        (exception_marker, true, pos)
    } else if let Some(pos) = line.find(normal_marker) {
        (normal_marker, false, pos)
    } else {
        return None;
    };

    let domain_part = line[..marker_pos].trim();
    let start = marker_pos + marker.len();
    let end = line.rfind(')')?;
    if end < start {
        return None;
    }

    let scriptlet_raw = line[start..end].trim();
    if scriptlet_raw.is_empty() && !is_exception {
        return None;
    }

    let mut rule = make_special_rule();
    rule.domain_constraints = parse_cosmetic_domains(domain_part);
    rule.scriptlet = Some(ScriptletRule {
        scriptlet: scriptlet_raw.to_string(),
        is_exception,
        is_generic: domain_part.is_empty(),
    });
    Some(rule)
}

fn is_procedural_selector(selector: &str) -> bool {
    let lower = selector.to_ascii_lowercase();
    lower.contains(":has-text(")
        || lower.contains(":matches-css(")
        || lower.contains(":xpath(")
        || lower.contains(":upward(")
        || lower.contains(":remove(")
        || lower.contains(":style(")
}

fn parse_procedural_line(line: &str) -> Option<CompiledRule> {
    let exception_marker = "#@?#";
    let normal_marker = "#?#";

    let (marker, is_exception, marker_pos) = if let Some(pos) = line.find(exception_marker) {
        (exception_marker, true, pos)
    } else if let Some(pos) = line.find(normal_marker) {
        (normal_marker, false, pos)
    } else if let Some(pos) = line.find("#@#") {
        let selector = line[pos + 3..].trim();
        if is_procedural_selector(selector) {
            ("#@#", true, pos)
        } else {
            return None;
        }
    } else if let Some(pos) = line.find("##") {
        let selector = line[pos + 2..].trim();
        if is_procedural_selector(selector) {
            ("##", false, pos)
        } else {
            return None;
        }
    } else {
        return None;
    };

    let domain_part = line[..marker_pos].trim();
    let selector = line[marker_pos + marker.len()..].trim();
    if selector.is_empty() || selector.starts_with("+js(") {
        return None;
    }
    if !is_procedural_selector(selector) {
        return None;
    }

    let mut rule = make_special_rule();
    rule.domain_constraints = parse_cosmetic_domains(domain_part);
    rule.procedural = Some(ProceduralRule {
        selector: selector.to_string(),
        is_exception,
        is_generic: domain_part.is_empty(),
    });
    Some(rule)
}

fn parse_cosmetic_line(line: &str) -> Option<CompiledRule> {
    let exception_marker = "#@#";
    let normal_marker = "##";

    let (marker, is_exception, marker_pos) = if let Some(pos) = line.find(exception_marker) {
        (exception_marker, true, pos)
    } else if let Some(pos) = line.find(normal_marker) {
        (normal_marker, false, pos)
    } else {
        return None;
    };

    let domain_part = line[..marker_pos].trim();
    let selector = line[marker_pos + marker.len()..].trim();
    if selector.is_empty() {
        return None;
    }

    if selector.starts_with('^') {
        return None;
    }

    if selector.starts_with("+js(") {
        return None;
    }

    if selector.contains(":has-text(")
        || selector.contains(":matches-css(")
        || selector.contains(":xpath(")
        || selector.contains(":upward(")
        || selector.contains(":remove(")
        || selector.contains(":style(")
    {
        return None;
    }

    let mut rule = make_special_rule();
    rule.domain_constraints = parse_cosmetic_domains(domain_part);
    rule.cosmetic = Some(CosmeticRule {
        selector: selector.to_string(),
        is_exception,
        is_generic: domain_part.is_empty(),
    });
    Some(rule)
}

struct ParsedPattern {
    domain: String,
    pattern: String,
    anchor_type: AnchorType,
}

fn parse_pattern_rule(line: &str) -> Option<ParsedPattern> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let (anchor_type, rest) = if line.starts_with("||") {
        (AnchorType::Hostname, &line[2..])
    } else if line.starts_with('|') {
        (AnchorType::Left, &line[1..])
    } else {
        (AnchorType::None, line)
    };

    let rest = rest.trim_end_matches('|');

    if rest.is_empty() || rest.starts_with('/') && !rest.contains('.') {
        return None;
    }

    let domain = extract_pattern_domain(rest, anchor_type);

    Some(ParsedPattern {
        domain,
        pattern: rest.to_string(),
        anchor_type,
    })
}

fn extract_pattern_domain(pattern: &str, anchor_type: AnchorType) -> String {
    if anchor_type != AnchorType::Hostname {
        return String::new();
    }

    let mut end = pattern.len();
    for (i, ch) in pattern.char_indices() {
        if ch == '/' || ch == '^' || ch == '*' || ch == '?' || ch == '#' {
            end = i;
            break;
        }
    }

    let host_part = &pattern[..end];
    normalize_domain(host_part).unwrap_or_default()
}
