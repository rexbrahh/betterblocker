use std::collections::HashSet;

use crate::parser::CompiledRule;

pub fn optimize_rules(rules: &mut Vec<CompiledRule>) {
    let mut seen: HashSet<RuleKey> = HashSet::new();
    rules.retain(|rule| {
        let key = RuleKey::from(rule);
        if seen.contains(&key) {
            false
        } else {
            seen.insert(key);
            true
        }
    });
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
        }
    }
}
