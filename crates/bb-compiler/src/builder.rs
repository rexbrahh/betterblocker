use std::collections::HashMap;

use bb_core::hash::{hash_domain, hash_token, Hash64};
use bb_core::snapshot::{
    align_offset, header, section_entry, SectionId, HEADER_SIZE, SECTION_ENTRY_SIZE, UBX_MAGIC,
    UBX_VERSION, HASHMAP64_ENTRY_SIZE, HASHMAP64_HEADER_SIZE, NO_CONSTRAINT, NO_PATTERN,
    TOKEN_DICT_HEADER_SIZE, TOKEN_DICT_ENTRY_SIZE, PatternOp,
};
use bb_core::types::RuleAction;

use crate::parser::{AnchorType, CompiledRule};

const HASH_SEED_LO: u32 = 0x9e3779b9;
const HASH_SEED_HI: u32 = 0x85ebca6b;

pub fn build_snapshot(rules: &[CompiledRule]) -> Vec<u8> {
    let mut str_pool = StringPool::new();
    let domain_sets = build_domain_sets_section(rules);
    let (constraint_pool, constraint_offsets) = build_domain_constraint_pool(rules);
    
    let (pattern_pool, pattern_ids) = build_pattern_pool(rules, &mut str_pool);
    let (token_dict, token_postings) = build_token_sections(rules, &pattern_ids);
    
    let rules_section = build_rules_section(rules, &constraint_offsets, &pattern_ids);
    let str_pool_section = str_pool.build();

    let mut sections = vec![
        SectionData::new(SectionId::StrPool, str_pool_section),
        SectionData::new(SectionId::DomainSets, domain_sets),
        SectionData::new(SectionId::TokenDict, token_dict),
        SectionData::new(SectionId::TokenPostings, token_postings),
        SectionData::new(SectionId::PatternPool, pattern_pool),
        SectionData::new(SectionId::DomainConstraintPool, constraint_pool),
        SectionData::new(SectionId::Rules, rules_section),
    ];

    let section_count = sections.len();
    let section_dir_offset = HEADER_SIZE;
    let section_dir_bytes = section_count * SECTION_ENTRY_SIZE;
    let mut data_offset = align_offset(section_dir_offset + section_dir_bytes, 4);

    for section in &mut sections {
        section.offset = data_offset;
        data_offset = align_offset(data_offset + section.data.len(), 4);
    }

    let total_size = data_offset;
    let mut buffer = vec![0u8; total_size];

    buffer[0..4].copy_from_slice(&UBX_MAGIC);
    write_u16_le(&mut buffer, header::VERSION, UBX_VERSION);
    write_u16_le(&mut buffer, header::FLAGS, 0);
    write_u32_le(&mut buffer, header::HEADER_BYTES, HEADER_SIZE as u32);
    write_u32_le(&mut buffer, header::SECTION_COUNT, section_count as u32);
    write_u32_le(&mut buffer, header::SECTION_DIR_OFFSET, section_dir_offset as u32);
    write_u32_le(&mut buffer, header::SECTION_DIR_BYTES, section_dir_bytes as u32);
    write_u32_le(&mut buffer, header::BUILD_ID, 0);

    for (index, section) in sections.iter().enumerate() {
        let entry_offset = section_dir_offset + index * SECTION_ENTRY_SIZE;
        write_u16_le(&mut buffer, entry_offset + section_entry::ID, section.id as u16);
        write_u16_le(&mut buffer, entry_offset + section_entry::FLAGS, 0);
        write_u32_le(&mut buffer, entry_offset + section_entry::OFFSET, section.offset as u32);
        write_u32_le(&mut buffer, entry_offset + section_entry::LENGTH, section.data.len() as u32);
        write_u32_le(&mut buffer, entry_offset + section_entry::UNCOMPRESSED_LENGTH, 0);
        write_u32_le(&mut buffer, entry_offset + section_entry::CRC32, 0);

        let end = section.offset + section.data.len();
        buffer[section.offset..end].copy_from_slice(&section.data);
    }

    buffer
}

struct SectionData {
    id: SectionId,
    data: Vec<u8>,
    offset: usize,
}

impl SectionData {
    fn new(id: SectionId, data: Vec<u8>) -> Self {
        Self { id, data, offset: 0 }
    }
}

struct StringPool {
    data: Vec<u8>,
    index: HashMap<String, u32>,
}

impl StringPool {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn intern(&mut self, s: &str) -> (u32, u16) {
        if let Some(&offset) = self.index.get(s) {
            return (offset, s.len() as u16);
        }
        let offset = self.data.len() as u32;
        self.data.extend_from_slice(s.as_bytes());
        self.index.insert(s.to_string(), offset);
        (offset, s.len() as u16)
    }

    fn build(self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.data.len());
        buf.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.data);
        buf
    }
}

fn build_domain_sets_section(rules: &[CompiledRule]) -> Vec<u8> {
    let mut block_map: HashMap<Hash64, u32> = HashMap::new();
    let mut allow_map: HashMap<Hash64, u32> = HashMap::new();

    for (rule_id, rule) in rules.iter().enumerate() {
        if rule.pattern.is_some() {
            continue;
        }
        if rule.action != RuleAction::Block && rule.action != RuleAction::Allow {
            continue;
        }
        if rule.domain.is_empty() {
            continue;
        }
        let hash = hash_domain(&rule.domain);
        let target = match rule.action {
            RuleAction::Block => &mut block_map,
            RuleAction::Allow => &mut allow_map,
            _ => continue,
        };
        target.insert(hash, rule_id as u32);
    }

    let block_entries = map_to_entries(&block_map);
    let allow_entries = map_to_entries(&allow_map);

    let block_bytes = build_hashmap64(&block_entries);
    let allow_bytes = build_hashmap64(&allow_entries);

    let mut section = Vec::with_capacity(block_bytes.len() + allow_bytes.len());
    section.extend_from_slice(&block_bytes);
    section.extend_from_slice(&allow_bytes);
    section
}

fn map_to_entries(map: &HashMap<Hash64, u32>) -> Vec<(Hash64, u32)> {
    map.iter().map(|(hash, value)| (*hash, *value)).collect()
}

fn build_domain_constraint_pool(rules: &[CompiledRule]) -> (Vec<u8>, Vec<u32>) {
    let mut pool = Vec::new();
    pool.extend_from_slice(&0u32.to_le_bytes());

    let mut offsets = Vec::with_capacity(rules.len());

    for rule in rules {
        match &rule.domain_constraints {
            Some(constraints) if !constraints.include.is_empty() || !constraints.exclude.is_empty() => {
                let offset = pool.len() - 4;
                offsets.push(offset as u32);

                pool.extend_from_slice(&(constraints.include.len() as u16).to_le_bytes());
                pool.extend_from_slice(&(constraints.exclude.len() as u16).to_le_bytes());

                for hash in &constraints.include {
                    pool.extend_from_slice(&hash.lo.to_le_bytes());
                    pool.extend_from_slice(&hash.hi.to_le_bytes());
                }

                for hash in &constraints.exclude {
                    pool.extend_from_slice(&hash.lo.to_le_bytes());
                    pool.extend_from_slice(&hash.hi.to_le_bytes());
                }
            }
            _ => {
                offsets.push(NO_CONSTRAINT);
            }
        }
    }

    let pool_len = (pool.len() - 4) as u32;
    pool[0..4].copy_from_slice(&pool_len.to_le_bytes());

    (pool, offsets)
}

fn build_pattern_pool(rules: &[CompiledRule], str_pool: &mut StringPool) -> (Vec<u8>, Vec<u32>) {
    let mut pattern_ids = Vec::with_capacity(rules.len());
    let mut pattern_entries: Vec<PatternEntry> = Vec::new();
    let mut prog_bytes: Vec<u8> = Vec::new();

    for rule in rules {
        if let Some(pattern) = &rule.pattern {
            let (bytecode, host_hash) = compile_pattern(pattern, rule.anchor_type, str_pool);
            
            let prog_offset = prog_bytes.len() as u32;
            prog_bytes.extend_from_slice(&bytecode);
            
            let pattern_id = pattern_entries.len() as u32;
            pattern_entries.push(PatternEntry {
                prog_offset,
                prog_len: bytecode.len() as u16,
                anchor_type: match rule.anchor_type {
                    AnchorType::None => 0,
                    AnchorType::Left => 1,
                    AnchorType::Hostname => 2,
                },
                flags: 0,
                host_hash_lo: host_hash.lo,
                host_hash_hi: host_hash.hi,
            });
            pattern_ids.push(pattern_id);
        } else {
            pattern_ids.push(NO_PATTERN);
        }
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(pattern_entries.len() as u32).to_le_bytes());
    
    for entry in &pattern_entries {
        section.extend_from_slice(&entry.prog_offset.to_le_bytes());
        section.extend_from_slice(&entry.prog_len.to_le_bytes());
        section.push(entry.anchor_type);
        section.push(entry.flags);
        section.extend_from_slice(&entry.host_hash_lo.to_le_bytes());
        section.extend_from_slice(&entry.host_hash_hi.to_le_bytes());
        section.extend_from_slice(&[0u8; 8]);
    }
    
    section.extend_from_slice(&(prog_bytes.len() as u32).to_le_bytes());
    section.extend_from_slice(&prog_bytes);

    (section, pattern_ids)
}

struct PatternEntry {
    prog_offset: u32,
    prog_len: u16,
    anchor_type: u8,
    flags: u8,
    host_hash_lo: u32,
    host_hash_hi: u32,
}

fn compile_pattern(pattern: &str, anchor_type: AnchorType, str_pool: &mut StringPool) -> (Vec<u8>, Hash64) {
    let mut bytecode = Vec::new();
    let mut host_hash = Hash64 { lo: 0, hi: 0 };
    let pattern_lower = pattern.to_lowercase();
    
    if anchor_type == AnchorType::Hostname {
        bytecode.push(PatternOp::HostAnchor as u8);
        
        if let Some(end) = pattern_lower.find(|c| c == '/' || c == '^' || c == '*') {
            let host = &pattern_lower[..end];
            if !host.is_empty() {
                host_hash = hash_domain(host);
            }
        } else {
            host_hash = hash_domain(&pattern_lower);
        }
    } else if anchor_type == AnchorType::Left {
        bytecode.push(PatternOp::AssertStart as u8);
    }

    let mut chars = pattern_lower.chars().peekable();
    let mut literal_start = None;
    let mut pos = 0;

    while let Some(ch) = chars.next() {
        match ch {
            '*' => {
                if let Some(start) = literal_start.take() {
                    emit_literal(&mut bytecode, &pattern_lower[start..pos], str_pool);
                }
                bytecode.push(PatternOp::SkipAny as u8);
            }
            '^' => {
                if let Some(start) = literal_start.take() {
                    emit_literal(&mut bytecode, &pattern_lower[start..pos], str_pool);
                }
                bytecode.push(PatternOp::AssertBoundary as u8);
            }
            _ => {
                if literal_start.is_none() {
                    literal_start = Some(pos);
                }
            }
        }
        pos += ch.len_utf8();
    }

    if let Some(start) = literal_start {
        emit_literal(&mut bytecode, &pattern_lower[start..], str_pool);
    }

    bytecode.push(PatternOp::Done as u8);
    (bytecode, host_hash)
}

fn emit_literal(bytecode: &mut Vec<u8>, literal: &str, str_pool: &mut StringPool) {
    if literal.is_empty() {
        return;
    }
    let (offset, len) = str_pool.intern(literal);
    bytecode.push(PatternOp::FindLit as u8);
    bytecode.extend_from_slice(&offset.to_le_bytes());
    bytecode.extend_from_slice(&len.to_le_bytes());
}

fn build_token_sections(rules: &[CompiledRule], pattern_ids: &[u32]) -> (Vec<u8>, Vec<u8>) {
    let mut token_to_rules: HashMap<u32, Vec<u32>> = HashMap::new();

    for (rule_id, rule) in rules.iter().enumerate() {
        if pattern_ids[rule_id] == NO_PATTERN {
            continue;
        }
        
        if let Some(pattern) = &rule.pattern {
            let tokens = extract_pattern_tokens(pattern);
            for token_hash in tokens {
                token_to_rules.entry(token_hash).or_default().push(rule_id as u32);
            }
        }
    }

    if token_to_rules.is_empty() {
        let empty_dict = build_token_dict(&[]);
        let empty_postings = vec![0u8; 4];
        return (empty_dict, empty_postings);
    }

    let mut postings_data = Vec::new();
    let mut dict_entries: Vec<(u32, u32, u32)> = Vec::new();

    for (token_hash, rule_ids) in &token_to_rules {
        let postings_offset = postings_data.len() as u32;
        encode_posting_list(&mut postings_data, rule_ids);
        dict_entries.push((*token_hash, postings_offset, rule_ids.len() as u32));
    }

    let token_dict = build_token_dict(&dict_entries);
    
    let mut postings_section = Vec::new();
    postings_section.extend_from_slice(&(postings_data.len() as u32).to_le_bytes());
    postings_section.extend_from_slice(&postings_data);

    (token_dict, postings_section)
}

fn extract_pattern_tokens(pattern: &str) -> Vec<u32> {
    let mut tokens = Vec::new();
    let pattern_lower = pattern.to_lowercase();
    let bytes = pattern_lower.as_bytes();
    
    let mut token_start = None;
    
    for i in 0..=bytes.len() {
        let is_alnum = i < bytes.len() && bytes[i].is_ascii_alphanumeric();
        
        if is_alnum {
            if token_start.is_none() {
                token_start = Some(i);
            }
        } else if let Some(start) = token_start.take() {
            let len = i - start;
            if len >= 3 {
                let token_str = &pattern_lower[start..i];
                tokens.push(hash_token(token_str));
            }
        }
    }
    
    tokens
}

fn encode_posting_list(buf: &mut Vec<u8>, rule_ids: &[u32]) {
    let mut prev = 0u32;
    for &id in rule_ids {
        let delta = id.wrapping_sub(prev);
        encode_varint(buf, delta);
        prev = id;
    }
}

fn encode_varint(buf: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            buf.push(byte);
            break;
        } else {
            buf.push(byte | 0x80);
        }
    }
}

fn build_token_dict(entries: &[(u32, u32, u32)]) -> Vec<u8> {
    let count = entries.len();
    let capacity = if count == 0 { 0 } else { compute_capacity(count) };

    let mut buf = vec![0u8; TOKEN_DICT_HEADER_SIZE + capacity * TOKEN_DICT_ENTRY_SIZE];
    write_u32_le(&mut buf, 0, capacity as u32);
    write_u32_le(&mut buf, 4, count as u32);
    write_u32_le(&mut buf, 8, HASH_SEED_LO);
    write_u32_le(&mut buf, 12, 0);

    if capacity == 0 {
        return buf;
    }

    let entries_offset = TOKEN_DICT_HEADER_SIZE;
    let mask = capacity - 1;

    for &(token_hash, postings_off, rule_count) in entries {
        let mut idx = (token_hash as usize) & mask;
        for _ in 0..capacity {
            let entry_offset = entries_offset + idx * TOKEN_DICT_ENTRY_SIZE;
            let stored = read_u32_le(&buf, entry_offset);
            if stored == 0 {
                write_u32_le(&mut buf, entry_offset, token_hash);
                write_u32_le(&mut buf, entry_offset + 4, postings_off);
                write_u32_le(&mut buf, entry_offset + 8, rule_count);
                break;
            }
            idx = (idx + 1) & mask;
        }
    }

    buf
}

fn build_rules_section(rules: &[CompiledRule], constraint_offsets: &[u32], pattern_ids: &[u32]) -> Vec<u8> {
    let count = rules.len();
    let mut buf = Vec::new();
    buf.extend_from_slice(&(count as u32).to_le_bytes());

    if count == 0 {
        return buf;
    }

    let mut pos = 4;

    pad_to(&mut buf, pos);
    for rule in rules {
        buf.push(rule.action as u8);
    }
    pos += count;
    pos = align_offset(pos, 2);
    pad_to(&mut buf, pos);

    for rule in rules {
        buf.extend_from_slice(&rule.flags.bits().to_le_bytes());
    }
    pos += count * 2;
    pos = align_offset(pos, 4);
    pad_to(&mut buf, pos);

    for rule in rules {
        buf.extend_from_slice(&rule.type_mask.bits().to_le_bytes());
    }
    pos += count * 4;
    pad_to(&mut buf, pos);

    for rule in rules {
        buf.push(rule.party_mask.bits());
    }
    pos += count;
    pos = align_offset(pos, 1);
    pad_to(&mut buf, pos);

    for rule in rules {
        buf.push(rule.scheme_mask.bits());
    }
    pos += count;
    pos = align_offset(pos, 4);
    pad_to(&mut buf, pos);

    for pattern_id in pattern_ids {
        buf.extend_from_slice(&pattern_id.to_le_bytes());
    }
    pos += count * 4;
    pad_to(&mut buf, pos);

    for offset in constraint_offsets {
        buf.extend_from_slice(&offset.to_le_bytes());
    }
    pos += count * 4;
    pad_to(&mut buf, pos);

    for _ in rules {
        buf.extend_from_slice(&0u32.to_le_bytes());
    }
    pos += count * 4;
    pad_to(&mut buf, pos);

    for _ in rules {
        buf.extend_from_slice(&0i16.to_le_bytes());
    }
    pos += count * 2;
    pos = align_offset(pos, 2);
    pad_to(&mut buf, pos);

    for rule in rules {
        buf.extend_from_slice(&rule.list_id.to_le_bytes());
    }

    buf
}

fn build_hashmap64(entries: &[(Hash64, u32)]) -> Vec<u8> {
    let count = entries.len();
    let capacity = if count == 0 { 0 } else { compute_capacity(count) };

    let mut buf = vec![0u8; HASHMAP64_HEADER_SIZE + capacity * HASHMAP64_ENTRY_SIZE];
    write_u32_le(&mut buf, 0, capacity as u32);
    write_u32_le(&mut buf, 4, count as u32);
    write_u32_le(&mut buf, 8, HASH_SEED_LO);
    write_u32_le(&mut buf, 12, HASH_SEED_HI);
    write_u32_le(&mut buf, 16, 0);

    if capacity == 0 {
        return buf;
    }

    let entries_offset = HASHMAP64_HEADER_SIZE;
    let mask = capacity - 1;

    for (hash, value) in entries {
        let mut idx = (hash.lo as usize) & mask;
        for _ in 0..capacity {
            let entry_offset = entries_offset + idx * HASHMAP64_ENTRY_SIZE;
            let lo = read_u32_le(&buf, entry_offset);
            let hi = read_u32_le(&buf, entry_offset + 4);
            if lo == 0 && hi == 0 {
                write_u32_le(&mut buf, entry_offset, hash.lo);
                write_u32_le(&mut buf, entry_offset + 4, hash.hi);
                write_u32_le(&mut buf, entry_offset + 8, *value);
                break;
            }
            idx = (idx + 1) & mask;
        }
    }

    buf
}

fn compute_capacity(count: usize) -> usize {
    let target = ((count as f64) / 0.7).ceil() as usize;
    let mut capacity = 1usize;
    while capacity < target {
        capacity <<= 1;
    }
    capacity.max(2)
}

fn pad_to(buf: &mut Vec<u8>, target_len: usize) {
    if buf.len() < target_len {
        buf.resize(target_len, 0);
    }
}

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn write_u16_le(data: &mut [u8], offset: usize, value: u16) {
    let bytes = value.to_le_bytes();
    data[offset..offset + 2].copy_from_slice(&bytes);
}

fn write_u32_le(data: &mut [u8], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    data[offset..offset + 4].copy_from_slice(&bytes);
}

#[cfg(test)]
mod tests {
    use bb_core::hash::hash_domain;
    use bb_core::matcher::Matcher;
    use bb_core::snapshot::Snapshot;
    use bb_core::types::{MatchDecision, RequestContext, RequestType, SchemeMask};

    use crate::parser::parse_filter_list;

    use super::build_snapshot;

    #[test]
    fn builds_domain_sets_and_rules() {
        let rules = parse_filter_list("||example.com^\n||ads.example.com^\n@@||ads.example.com^");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");

        let ads_hash = hash_domain("ads.example.com");
        assert!(snapshot.domain_block_set().contains(ads_hash));
        assert!(snapshot.domain_allow_set().contains(ads_hash));

        let matcher = Matcher::new(&snapshot);
        let ctx = RequestContext {
            url: "https://ads.example.com/script.js",
            req_host: "ads.example.com",
            req_etld1: "example.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: false,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx);
        assert_eq!(result.decision, MatchDecision::Allow);
    }

    #[test]
    fn applies_domain_rule_options() {
        let rules = parse_filter_list("||ads.example.com^$script,third-party");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx_block = RequestContext {
            url: "https://ads.example.com/script.js",
            req_host: "ads.example.com",
            req_etld1: "example.com",
            site_host: "site.com",
            site_etld1: "site.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx_block);
        assert_eq!(result.decision, MatchDecision::Block);

        let ctx_allow = RequestContext {
            url: "https://ads.example.com/image.png",
            req_host: "ads.example.com",
            req_etld1: "example.com",
            site_host: "site.com",
            site_etld1: "site.com",
            is_third_party: true,
            request_type: RequestType::IMAGE,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "1",
        };

        let result = matcher.match_request(&ctx_allow);
        assert_eq!(result.decision, MatchDecision::Allow);

        let ctx_first_party = RequestContext {
            url: "https://ads.example.com/script.js",
            req_host: "ads.example.com",
            req_etld1: "example.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: false,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "2",
        };

        let result = matcher.match_request(&ctx_first_party);
        assert_eq!(result.decision, MatchDecision::Allow);
    }

    #[test]
    fn applies_domain_constraints() {
        let rules = parse_filter_list("||ads.example.com^$domain=site.com");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx_match = RequestContext {
            url: "https://ads.example.com/script.js",
            req_host: "ads.example.com",
            req_etld1: "example.com",
            site_host: "site.com",
            site_etld1: "site.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx_match);
        assert_eq!(result.decision, MatchDecision::Block);

        let ctx_no_match = RequestContext {
            url: "https://ads.example.com/script.js",
            req_host: "ads.example.com",
            req_etld1: "example.com",
            site_host: "other.com",
            site_etld1: "other.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "1",
        };

        let result = matcher.match_request(&ctx_no_match);
        assert_eq!(result.decision, MatchDecision::Allow);
    }

    #[test]
    fn applies_domain_exclusions() {
        let rules = parse_filter_list("||ads.example.com^$domain=~safe.com");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx_blocked = RequestContext {
            url: "https://ads.example.com/script.js",
            req_host: "ads.example.com",
            req_etld1: "example.com",
            site_host: "other.com",
            site_etld1: "other.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx_blocked);
        assert_eq!(result.decision, MatchDecision::Block);

        let ctx_allowed = RequestContext {
            url: "https://ads.example.com/script.js",
            req_host: "ads.example.com",
            req_etld1: "example.com",
            site_host: "safe.com",
            site_etld1: "safe.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "1",
        };

        let result = matcher.match_request(&ctx_allowed);
        assert_eq!(result.decision, MatchDecision::Allow);
    }

    #[test]
    fn matches_url_pattern_rules() {
        let rules = parse_filter_list("||example.com/ads/*\n||tracker.com/pixel.gif");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx_match = RequestContext {
            url: "https://example.com/ads/banner.js",
            req_host: "example.com",
            req_etld1: "example.com",
            site_host: "other.com",
            site_etld1: "other.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx_match);
        assert_eq!(result.decision, MatchDecision::Block);

        let ctx_no_match = RequestContext {
            url: "https://example.com/content/page.html",
            req_host: "example.com",
            req_etld1: "example.com",
            site_host: "other.com",
            site_etld1: "other.com",
            is_third_party: true,
            request_type: RequestType::MAIN_FRAME,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "1",
        };

        let result = matcher.match_request(&ctx_no_match);
        assert_eq!(result.decision, MatchDecision::Allow);
    }

    #[test]
    fn matches_plain_pattern_rules() {
        let rules = parse_filter_list("/analytics.js");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx_match = RequestContext {
            url: "https://cdn.example.com/analytics.js",
            req_host: "cdn.example.com",
            req_etld1: "example.com",
            site_host: "site.com",
            site_etld1: "site.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx_match);
        assert_eq!(result.decision, MatchDecision::Block);
    }
}
