use std::collections::HashSet;

use crate::parser::CompiledRule;

pub struct OptimizeStats {
    pub before: usize,
    pub after: usize,
    pub deduped: usize,
    pub badfilter_rules: usize,
    pub badfiltered_rules: usize,
}

pub fn optimize_rules(rules: &mut Vec<CompiledRule>) -> OptimizeStats {
    let before = rules.len();
    let mut badfilter_keys: HashSet<BadfilterKey> = HashSet::new();
    let mut badfilter_rules = 0usize;

    for rule in rules.iter() {
        if rule.is_badfilter {
            badfilter_rules += 1;
            badfilter_keys.insert(BadfilterKey::from(rule));
        }
    }

    let mut badfiltered_rules = 0usize;
    if !badfilter_keys.is_empty() {
        rules.retain(|rule| {
            if rule.is_badfilter {
                return false;
            }
            if badfilter_keys.contains(&BadfilterKey::from(rule)) {
                badfiltered_rules += 1;
                return false;
            }
            true
        });
    } else {
        rules.retain(|rule| !rule.is_badfilter);
    }

    let mut seen: HashSet<RuleKey> = HashSet::new();
    let mut deduped = 0usize;
    rules.retain(|rule| {
        let key = RuleKey::from(rule);
        if seen.contains(&key) {
            deduped += 1;
            false
        } else {
            seen.insert(key);
            true
        }
    });

    let after = rules.len();

    OptimizeStats {
        before,
        after,
        deduped,
        badfilter_rules,
        badfiltered_rules,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuleKey {
    action: u8,
    flags: u16,
    type_mask: u32,
    party_mask: u8,
    scheme_mask: u8,
    list_id: u16,
    domain: String,
    pattern: Option<String>,
    anchor_type: u8,
    constraint_include: Vec<u64>,
    constraint_exclude: Vec<u64>,
    redirect: Option<String>,
    removeparam: Option<String>,
    csp: Option<String>,
    header: Option<crate::parser::HeaderSpec>,
    cosmetic: Option<crate::parser::CosmeticRule>,
    scriptlet: Option<crate::parser::ScriptletRule>,
    responseheader: Option<crate::parser::ResponseHeaderRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BadfilterKey {
    action: u8,
    flags: u16,
    type_mask: u32,
    party_mask: u8,
    scheme_mask: u8,
    domain: String,
    pattern: Option<String>,
    anchor_type: u8,
    constraint_include: Vec<u64>,
    constraint_exclude: Vec<u64>,
    redirect: Option<String>,
    removeparam: Option<String>,
    csp: Option<String>,
    header: Option<crate::parser::HeaderSpec>,
    cosmetic: Option<crate::parser::CosmeticRule>,
    scriptlet: Option<crate::parser::ScriptletRule>,
    responseheader: Option<crate::parser::ResponseHeaderRule>,
}

impl From<&CompiledRule> for RuleKey {
    fn from(rule: &CompiledRule) -> Self {
        let (include, exclude) = match &rule.domain_constraints {
            Some(c) => (
                c.include.iter().map(|h| h.to_u64()).collect(),
                c.exclude.iter().map(|h| h.to_u64()).collect(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        Self {
            action: rule.action as u8,
            flags: rule.flags.bits(),
            type_mask: rule.type_mask.bits(),
            party_mask: rule.party_mask.bits(),
            scheme_mask: rule.scheme_mask.bits(),
            list_id: rule.list_id,
            domain: rule.domain.clone(),
            pattern: rule.pattern.clone(),
            anchor_type: rule.anchor_type as u8,
            constraint_include: include,
            constraint_exclude: exclude,
            redirect: rule.redirect.clone(),
            removeparam: rule.removeparam.clone(),
            csp: rule.csp.clone(),
            header: rule.header.clone(),
            cosmetic: rule.cosmetic.clone(),
            scriptlet: rule.scriptlet.clone(),
            responseheader: rule.responseheader.clone(),
        }
    }
}

impl From<&CompiledRule> for BadfilterKey {
    fn from(rule: &CompiledRule) -> Self {
        let (include, exclude) = match &rule.domain_constraints {
            Some(c) => (
                c.include.iter().map(|h| h.to_u64()).collect(),
                c.exclude.iter().map(|h| h.to_u64()).collect(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        Self {
            action: rule.action as u8,
            flags: rule.flags.bits(),
            type_mask: rule.type_mask.bits(),
            party_mask: rule.party_mask.bits(),
            scheme_mask: rule.scheme_mask.bits(),
            domain: rule.domain.clone(),
            pattern: rule.pattern.clone(),
            anchor_type: rule.anchor_type as u8,
            constraint_include: include,
            constraint_exclude: exclude,
            redirect: rule.redirect.clone(),
            removeparam: rule.removeparam.clone(),
            csp: rule.csp.clone(),
            header: rule.header.clone(),
            cosmetic: rule.cosmetic.clone(),
            scriptlet: rule.scriptlet.clone(),
            responseheader: rule.responseheader.clone(),
        }
    }
}
