use std::net::IpAddr;

use bb_core::hash::{hash_domain, Hash64};
use bb_core::psl::get_etld1;
use bb_core::types::{PartyMask, RequestType, RuleAction, RuleFlags, SchemeMask};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainConstraint {
    pub include: Vec<Hash64>,
    pub exclude: Vec<Hash64>,
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

        if line.contains("##") || line.contains("#@#") || line.contains("#?#") {
            continue;
        }

        let mut action = RuleAction::Block;
        if let Some(rest) = line.strip_prefix("@@") {
            action = RuleAction::Allow;
            line = rest.trim_start();
        }

        let (pattern_part, options_text) = split_rule_options(line);
        let options = match options_text {
            Some(options_text) => match parse_options(options_text) {
                Some(options) => options,
                None => continue,
            },
            None => ParsedOptions::default(),
        };

        let pattern_str = pattern_part.trim();

        if let Some(domain) = parse_host_anchor_rule(pattern_str) {
            let (final_action, final_flags) = apply_redirect_action(action, options.flags, options.redirect.is_some());
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
                redirect: options.redirect.clone(),
            });
            continue;
        }

        if let Some(domain) = parse_hosts_file_domain(pattern_str) {
            let (final_action, final_flags) = apply_redirect_action(action, options.flags, options.redirect.is_some());
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
                redirect: options.redirect.clone(),
            });
            continue;
        }

        if let Some(parsed) = parse_pattern_rule(pattern_str) {
            let (final_action, final_flags) = apply_redirect_action(action, options.flags, options.redirect.is_some());
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
                redirect: options.redirect,
            });
        }
    }

    rules
}

fn apply_redirect_action(action: RuleAction, flags: RuleFlags, has_redirect: bool) -> (RuleAction, RuleFlags) {
    if has_redirect && action == RuleAction::Block {
        (RuleAction::RedirectDirective, flags | RuleFlags::FROM_REDIRECT_EQ)
    } else {
        (action, flags)
    }
}

#[derive(Clone)]
struct ParsedOptions {
    flags: RuleFlags,
    type_mask: RequestType,
    party_mask: PartyMask,
    scheme_mask: SchemeMask,
    domain_constraints: Option<DomainConstraint>,
    redirect: Option<String>,
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

        if let Some(domain_value) = raw_lower.strip_prefix("domain=") {
            let parsed = parse_domain_option(domain_value)?;
            domain_constraints = Some(merge_constraints(domain_constraints, parsed));
            continue;
        }

        if let Some(redirect_value) = raw_lower.strip_prefix("redirect=") {
            if !redirect_value.is_empty() {
                redirect = Some(redirect_value.to_string());
            }
            continue;
        }

        if let Some(redirect_value) = raw_lower.strip_prefix("redirect-rule=") {
            if !redirect_value.is_empty() {
                redirect = Some(redirect_value.to_string());
            }
            continue;
        }

        let (negated, name) = match raw_lower.strip_prefix('~') {
            Some(rest) => (true, rest),
            None => (false, raw_lower),
        };

        if name.is_empty() || name.contains('=') || name == "badfilter" {
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
        let etld1 = get_etld1(&domain);
        let hash = hash_domain(&etld1);

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

fn is_comment_line(line: &str) -> bool {
    line.starts_with('!') || line.starts_with('[') || line.starts_with('#')
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
