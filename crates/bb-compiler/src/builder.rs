use std::collections::HashMap;

use bb_core::hash::{hash_domain, murmur3_32, Hash64};
use bb_core::snapshot::{
    align_offset, header, section_entry, SectionId, HEADER_SIZE, SECTION_ENTRY_SIZE, UBX_MAGIC,
    UBX_VERSION, HASHMAP64_ENTRY_SIZE, HASHMAP64_HEADER_SIZE, NO_CONSTRAINT, NO_PATTERN,
    TOKEN_DICT_HEADER_SIZE, TOKEN_DICT_ENTRY_SIZE, PatternOp,
};
use bb_core::types::RuleAction;

use crate::parser::{AnchorType, CompiledRule};

const HASH_SEED_LO: u32 = 0x9e3779b9;
const HASH_SEED_HI: u32 = 0x85ebca6b;
const NO_OPTION_ID: u32 = 0xFFFF_FFFF;

pub fn build_snapshot(rules: &[CompiledRule]) -> Vec<u8> {
    let mut str_pool = StringPool::new();
    let domain_sets = build_domain_sets_section(rules);
    let (constraint_pool, constraint_offsets) = build_domain_constraint_pool(rules);

    let (pattern_pool, pattern_ids) = build_pattern_pool(rules, &mut str_pool);
    let (token_dict, token_postings) = build_token_sections(rules, &pattern_ids);
    let (redirect_resources, redirect_option_ids) = build_redirect_resources_section(rules, &mut str_pool);
    let (removeparam_specs, removeparam_option_ids) =
        build_removeparam_specs_section(rules, &mut str_pool);
    let (csp_specs, csp_option_ids) = build_csp_specs_section(rules, &mut str_pool);
    let (header_specs, header_option_ids) = build_header_specs_section(rules, &mut str_pool);
    let responseheader_rules = build_responseheader_rules_section(rules, &constraint_offsets, &mut str_pool);
    let cosmetic_rules = build_cosmetic_rules_section(rules, &constraint_offsets, &mut str_pool);
    let procedural_rules = build_procedural_rules_section(rules, &constraint_offsets, &mut str_pool);
    let scriptlet_rules = build_scriptlet_rules_section(rules, &constraint_offsets, &mut str_pool);
    let option_ids = build_option_ids(
        rules,
        &redirect_option_ids,
        &removeparam_option_ids,
        &csp_option_ids,
        &header_option_ids,
    );

    let rules_section = build_rules_section(rules, &constraint_offsets, &pattern_ids, &option_ids);
    let str_pool_section = str_pool.build();

    let mut sections = vec![
        SectionData::new(SectionId::StrPool, str_pool_section),
        SectionData::new(SectionId::DomainSets, domain_sets),
        SectionData::new(SectionId::TokenDict, token_dict),
        SectionData::new(SectionId::TokenPostings, token_postings),
        SectionData::new(SectionId::PatternPool, pattern_pool),
        SectionData::new(SectionId::DomainConstraintPool, constraint_pool),
        SectionData::new(SectionId::RedirectResources, redirect_resources),
        SectionData::new(SectionId::RemoveparamSpecs, removeparam_specs),
        SectionData::new(SectionId::CspSpecs, csp_specs),
        SectionData::new(SectionId::HeaderSpecs, header_specs),
        SectionData::new(SectionId::ResponseHeaderRules, responseheader_rules),
        SectionData::new(SectionId::CosmeticRules, cosmetic_rules),
        SectionData::new(SectionId::ProceduralRules, procedural_rules),
        SectionData::new(SectionId::ScriptletRules, scriptlet_rules),
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
    let mut block_map: HashMap<Hash64, Vec<u32>> = HashMap::new();
    let mut allow_map: HashMap<Hash64, Vec<u32>> = HashMap::new();

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
        target.entry(hash).or_default().push(rule_id as u32);
    }

    let mut postings_data = Vec::new();
    let block_entries = map_to_posting_entries(&block_map, &mut postings_data);
    let allow_entries = map_to_posting_entries(&allow_map, &mut postings_data);

    let block_bytes = build_hashmap64(&block_entries);
    let allow_bytes = build_hashmap64(&allow_entries);

    let mut section = Vec::with_capacity(block_bytes.len() + allow_bytes.len() + postings_data.len() + 4);
    section.extend_from_slice(&block_bytes);
    section.extend_from_slice(&allow_bytes);
    section.extend_from_slice(&(postings_data.len() as u32).to_le_bytes());
    section.extend_from_slice(&postings_data);
    section
}

fn map_to_posting_entries(
    map: &HashMap<Hash64, Vec<u32>>,
    postings_data: &mut Vec<u8>,
) -> Vec<(Hash64, u32)> {
    map.iter()
        .map(|(hash, rule_ids)| {
            let offset = postings_data.len() as u32;
            encode_domain_posting_list(postings_data, rule_ids);
            (*hash, offset)
        })
        .collect()
}

fn encode_domain_posting_list(buf: &mut Vec<u8>, rule_ids: &[u32]) {
    buf.extend_from_slice(&(rule_ids.len() as u32).to_le_bytes());
    encode_posting_list(buf, rule_ids);
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
    let bytes = pattern.as_bytes();

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
                let token = &bytes[start..i];
                tokens.push(hash_token_bytes_lower(token));
            }
        }
    }

    tokens
}

fn hash_token_bytes_lower(token: &[u8]) -> u32 {
    let mut stack_buf = [0u8; 64];
    let bytes: &[u8] = if token.len() <= stack_buf.len() {
        for (i, &b) in token.iter().enumerate() {
            stack_buf[i] = b.to_ascii_lowercase();
        }
        &stack_buf[..token.len()]
    } else {
        let mut tmp = Vec::with_capacity(token.len());
        for &b in token {
            tmp.push(b.to_ascii_lowercase());
        }
        return hash_token_bytes(&tmp);
    };

    hash_token_bytes(bytes)
}

fn hash_token_bytes(bytes: &[u8]) -> u32 {
    let mut h = murmur3_32(bytes, 0x811c9dc5);
    if h == 0 {
        h = 1;
    }
    h
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

fn build_redirect_resources_section(
    rules: &[CompiledRule],
    str_pool: &mut StringPool,
) -> (Vec<u8>, Vec<u32>) {
    let mut option_ids = Vec::with_capacity(rules.len());
    let mut resources = Vec::new();
    let mut resource_index: HashMap<String, u32> = HashMap::new();

    for rule in rules {
        if let Some(redirect_name) = &rule.redirect {
            let index = if let Some(&existing) = resource_index.get(redirect_name) {
                existing
            } else {
                let path = redirect_resource_path(redirect_name);
                let (name_off, name_len) = str_pool.intern(redirect_name);
                let (path_off, path_len) = str_pool.intern(&path);
                let index = resources.len() as u32;
                resources.push(RedirectResource {
                    name_off,
                    name_len: name_len as u32,
                    path_off,
                    path_len: path_len as u32,
                });
                resource_index.insert(redirect_name.clone(), index);
                index
            };
            option_ids.push(index);
        } else {
            option_ids.push(NO_OPTION_ID);
        }
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(resources.len() as u32).to_le_bytes());
    for resource in &resources {
        section.extend_from_slice(&resource.name_off.to_le_bytes());
        section.extend_from_slice(&resource.name_len.to_le_bytes());
        section.extend_from_slice(&resource.path_off.to_le_bytes());
        section.extend_from_slice(&resource.path_len.to_le_bytes());
        section.extend_from_slice(&0u32.to_le_bytes());
    }

    (section, option_ids)
}

struct RedirectResource {
    name_off: u32,
    name_len: u32,
    path_off: u32,
    path_len: u32,
}

fn redirect_resource_path(name: &str) -> String {
    if name.starts_with('/') || name.starts_with("data:") || name.contains("://") {
        return name.to_string();
    }
    if name == "noopjs" {
        return "/redirects/noop.js".to_string();
    }
    if name.starts_with("redirects/") {
        return format!("/{}", name);
    }
    format!("/redirects/{}", name)
}

fn build_removeparam_specs_section(
    rules: &[CompiledRule],
    str_pool: &mut StringPool,
) -> (Vec<u8>, Vec<u32>) {
    let mut option_ids = Vec::with_capacity(rules.len());
    let mut specs = Vec::new();
    let mut spec_index: HashMap<String, u32> = HashMap::new();

    for rule in rules {
        if let Some(param) = &rule.removeparam {
            let index = if let Some(&existing) = spec_index.get(param) {
                existing
            } else {
                let (param_off, param_len) = str_pool.intern(param);
                let index = specs.len() as u32;
                specs.push(RemoveparamSpec {
                    param_off,
                    param_len: param_len as u32,
                });
                spec_index.insert(param.clone(), index);
                index
            };
            option_ids.push(index);
        } else {
            option_ids.push(NO_OPTION_ID);
        }
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(specs.len() as u32).to_le_bytes());
    for spec in &specs {
        section.extend_from_slice(&spec.param_off.to_le_bytes());
        section.extend_from_slice(&spec.param_len.to_le_bytes());
        section.extend_from_slice(&0u32.to_le_bytes());
    }

    (section, option_ids)
}

fn build_csp_specs_section(
    rules: &[CompiledRule],
    str_pool: &mut StringPool,
) -> (Vec<u8>, Vec<u32>) {
    let mut option_ids = Vec::with_capacity(rules.len());
    let mut specs = Vec::new();
    let mut spec_index: HashMap<String, u32> = HashMap::new();

    for rule in rules {
        if let Some(csp) = &rule.csp {
            let index = if let Some(&existing) = spec_index.get(csp) {
                existing
            } else {
                let (spec_off, spec_len) = str_pool.intern(csp);
                let index = specs.len() as u32;
                specs.push(CspSpec {
                    spec_off,
                    spec_len: spec_len as u32,
                });
                spec_index.insert(csp.clone(), index);
                index
            };
            option_ids.push(index);
        } else {
            option_ids.push(NO_OPTION_ID);
        }
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(specs.len() as u32).to_le_bytes());
    for spec in &specs {
        section.extend_from_slice(&spec.spec_off.to_le_bytes());
        section.extend_from_slice(&spec.spec_len.to_le_bytes());
        section.extend_from_slice(&0u32.to_le_bytes());
    }

    (section, option_ids)
}

fn build_header_specs_section(
    rules: &[CompiledRule],
    str_pool: &mut StringPool,
) -> (Vec<u8>, Vec<u32>) {
    let mut option_ids = Vec::with_capacity(rules.len());
    let mut specs = Vec::new();
    let mut spec_index: HashMap<crate::parser::HeaderSpec, u32> = HashMap::new();

    for rule in rules {
        if let Some(spec) = &rule.header {
            let index = if let Some(&existing) = spec_index.get(spec) {
                existing
            } else {
                let (name_off, name_len) = str_pool.intern(&spec.name);
                let (value_off, value_len) = match &spec.value {
                    Some(value) => {
                        let (off, len) = str_pool.intern(value);
                        (off, len as u32)
                    }
                    None => (0, 0),
                };
                let index = specs.len() as u32;
                specs.push(HeaderSpecEntry {
                    name_off,
                    name_len: name_len as u32,
                    value_off,
                    value_len,
                    flags: if spec.negate { 1 } else { 0 },
                });
                spec_index.insert(spec.clone(), index);
                index
            };
            option_ids.push(index);
        } else {
            option_ids.push(NO_OPTION_ID);
        }
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(specs.len() as u32).to_le_bytes());
    for spec in &specs {
        section.extend_from_slice(&spec.name_off.to_le_bytes());
        section.extend_from_slice(&spec.name_len.to_le_bytes());
        section.extend_from_slice(&spec.value_off.to_le_bytes());
        section.extend_from_slice(&spec.value_len.to_le_bytes());
        section.extend_from_slice(&spec.flags.to_le_bytes());
    }

    (section, option_ids)
}

fn build_responseheader_rules_section(
    rules: &[CompiledRule],
    constraint_offsets: &[u32],
    str_pool: &mut StringPool,
) -> Vec<u8> {
    let mut entries = Vec::new();

    for (idx, rule) in rules.iter().enumerate() {
        let responseheader = match &rule.responseheader {
            Some(rule) => rule,
            None => continue,
        };

        let (name_off, name_len) = str_pool.intern(&responseheader.header);
        let flags: u16 = if responseheader.is_exception { 1 } else { 0 };
        let list_id = rule.list_id;
        let constraint_offset = constraint_offsets.get(idx).copied().unwrap_or(NO_CONSTRAINT);

        entries.push((constraint_offset, name_off, name_len as u32, flags, list_id));
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (constraint_offset, name_off, name_len, flags, list_id) in entries {
        section.extend_from_slice(&constraint_offset.to_le_bytes());
        section.extend_from_slice(&name_off.to_le_bytes());
        section.extend_from_slice(&name_len.to_le_bytes());
        section.extend_from_slice(&flags.to_le_bytes());
        section.extend_from_slice(&list_id.to_le_bytes());
    }

    section
}

fn build_cosmetic_rules_section(
    rules: &[CompiledRule],
    constraint_offsets: &[u32],
    str_pool: &mut StringPool,
) -> Vec<u8> {
    let mut entries = Vec::new();

    for (idx, rule) in rules.iter().enumerate() {
        let cosmetic = match &rule.cosmetic {
            Some(rule) => rule,
            None => continue,
        };

        let (selector_off, selector_len) = str_pool.intern(&cosmetic.selector);
        let mut flags: u16 = 0;
        if cosmetic.is_exception {
            flags |= 1;
        }
        if cosmetic.is_generic {
            flags |= 1 << 1;
        }
        let list_id = rule.list_id;
        let constraint_offset = constraint_offsets.get(idx).copied().unwrap_or(NO_CONSTRAINT);

        entries.push((constraint_offset, selector_off, selector_len as u32, flags, list_id));
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (constraint_offset, selector_off, selector_len, flags, list_id) in entries {
        section.extend_from_slice(&constraint_offset.to_le_bytes());
        section.extend_from_slice(&selector_off.to_le_bytes());
        section.extend_from_slice(&selector_len.to_le_bytes());
        section.extend_from_slice(&flags.to_le_bytes());
        section.extend_from_slice(&list_id.to_le_bytes());
    }

    section
}

fn build_procedural_rules_section(
    rules: &[CompiledRule],
    constraint_offsets: &[u32],
    str_pool: &mut StringPool,
) -> Vec<u8> {
    let mut entries = Vec::new();

    for (idx, rule) in rules.iter().enumerate() {
        let procedural = match &rule.procedural {
            Some(rule) => rule,
            None => continue,
        };

        let (selector_off, selector_len) = str_pool.intern(&procedural.selector);
        let mut flags: u16 = 0;
        if procedural.is_exception {
            flags |= 1;
        }
        if procedural.is_generic {
            flags |= 1 << 1;
        }
        let list_id = rule.list_id;
        let constraint_offset = constraint_offsets.get(idx).copied().unwrap_or(NO_CONSTRAINT);

        entries.push((constraint_offset, selector_off, selector_len as u32, flags, list_id));
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (constraint_offset, selector_off, selector_len, flags, list_id) in entries {
        section.extend_from_slice(&constraint_offset.to_le_bytes());
        section.extend_from_slice(&selector_off.to_le_bytes());
        section.extend_from_slice(&selector_len.to_le_bytes());
        section.extend_from_slice(&flags.to_le_bytes());
        section.extend_from_slice(&list_id.to_le_bytes());
    }

    section
}

fn build_scriptlet_rules_section(
    rules: &[CompiledRule],
    constraint_offsets: &[u32],
    str_pool: &mut StringPool,
) -> Vec<u8> {
    let mut entries = Vec::new();

    for (idx, rule) in rules.iter().enumerate() {
        let scriptlet = match &rule.scriptlet {
            Some(rule) => rule,
            None => continue,
        };

        let (scriptlet_off, scriptlet_len) = str_pool.intern(&scriptlet.scriptlet);
        let mut flags: u16 = 0;
        if scriptlet.is_exception {
            flags |= 1;
        }
        if scriptlet.is_generic {
            flags |= 1 << 1;
        }
        let list_id = rule.list_id;
        let constraint_offset = constraint_offsets.get(idx).copied().unwrap_or(NO_CONSTRAINT);

        entries.push((constraint_offset, scriptlet_off, scriptlet_len as u32, flags, list_id));
    }

    let mut section = Vec::new();
    section.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (constraint_offset, scriptlet_off, scriptlet_len, flags, list_id) in entries {
        section.extend_from_slice(&constraint_offset.to_le_bytes());
        section.extend_from_slice(&scriptlet_off.to_le_bytes());
        section.extend_from_slice(&scriptlet_len.to_le_bytes());
        section.extend_from_slice(&flags.to_le_bytes());
        section.extend_from_slice(&list_id.to_le_bytes());
    }

    section
}

fn build_option_ids(
    rules: &[CompiledRule],
    redirect_option_ids: &[u32],
    removeparam_option_ids: &[u32],
    csp_option_ids: &[u32],
    header_option_ids: &[u32],
) -> Vec<u32> {
    let mut merged = Vec::with_capacity(rules.len());
    for (idx, rule) in rules.iter().enumerate() {
        let option_id = if rule.removeparam.is_some() {
            removeparam_option_ids.get(idx).copied().unwrap_or(NO_OPTION_ID)
        } else if rule.csp.is_some() {
            csp_option_ids.get(idx).copied().unwrap_or(NO_OPTION_ID)
        } else if rule.header.is_some() {
            header_option_ids.get(idx).copied().unwrap_or(NO_OPTION_ID)
        } else if rule.redirect.is_some() {
            redirect_option_ids.get(idx).copied().unwrap_or(NO_OPTION_ID)
        } else {
            NO_OPTION_ID
        };
        merged.push(option_id);
    }

    merged
}

struct RemoveparamSpec {
    param_off: u32,
    param_len: u32,
}

struct CspSpec {
    spec_off: u32,
    spec_len: u32,
}

struct HeaderSpecEntry {
    name_off: u32,
    name_len: u32,
    value_off: u32,
    value_len: u32,
    flags: u32,
}

fn build_rules_section(rules: &[CompiledRule], constraint_offsets: &[u32], pattern_ids: &[u32], option_ids: &[u32]) -> Vec<u8> {
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

    for offset in option_ids {
        buf.extend_from_slice(&offset.to_le_bytes());
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
    use bb_core::matcher::{Matcher, ResponseHeader};
    use bb_core::snapshot::Snapshot;
    use bb_core::types::{MatchDecision, RequestContext, RequestType, SchemeMask};

    use crate::optimizer::optimize_rules;
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

    #[test]
    fn applies_removeparam_rules() {
        let rules = parse_filter_list("||example.com^$removeparam=utm_source");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/path?utm_source=foo&x=1",
            req_host: "example.com",
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
        assert_eq!(result.decision, MatchDecision::Removeparam);
        assert_eq!(
            result.redirect_url.as_deref(),
            Some("https://example.com/path?x=1")
        );
    }

    #[test]
    fn removeparam_exception_disables_removal() {
        let rules = parse_filter_list(
            "||example.com^$removeparam=utm_source\n@@||example.com^$removeparam=utm_source",
        );
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/path?utm_source=foo&x=1",
            req_host: "example.com",
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
    fn injects_csp_and_respects_exceptions() {
        let rules = parse_filter_list("||example.com^$csp=script-src 'none'");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/index.html",
            req_host: "example.com",
            req_etld1: "example.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: false,
            request_type: RequestType::MAIN_FRAME,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let headers = [ResponseHeader {
            name: "Content-Type",
            value: "text/html",
        }];

        let result = matcher.match_response_headers(&ctx, &headers);
        assert_eq!(result.cancel, false);
        assert_eq!(result.csp_injections, vec!["script-src 'none'".to_string()]);

        let rules = parse_filter_list(
            "||example.com^$csp=script-src 'none'\n@@||example.com^$csp",
        );
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let result = matcher.match_response_headers(&ctx, &headers);
        assert!(result.csp_injections.is_empty());
    }

    #[test]
    fn header_rules_block_and_allow() {
        let rules = parse_filter_list("||example.com^$header=server:cloudflare");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/app.js",
            req_host: "example.com",
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

        let headers = [ResponseHeader {
            name: "Server",
            value: "cloudflare",
        }];

        let result = matcher.match_response_headers(&ctx, &headers);
        assert!(result.cancel);

        let rules = parse_filter_list(
            "||example.com^$header=server:cloudflare\n@@||example.com^$header=server:cloudflare",
        );
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let result = matcher.match_response_headers(&ctx, &headers);
        assert!(!result.cancel);
    }

    #[test]
    fn responseheader_removal_and_exception() {
        let rules = parse_filter_list("example.com##^responseheader(set-cookie)");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/index.html",
            req_host: "example.com",
            req_etld1: "example.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: false,
            request_type: RequestType::MAIN_FRAME,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let headers = [
            ResponseHeader {
                name: "Set-Cookie",
                value: "a=b",
            },
            ResponseHeader {
                name: "X-Test",
                value: "1",
            },
        ];

        let result = matcher.match_response_headers(&ctx, &headers);
        assert!(result.remove_headers.iter().any(|name| name == "set-cookie"));

        let rules = parse_filter_list(
            "example.com##^responseheader(set-cookie)\nexample.com#@#^responseheader(set-cookie)",
        );
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let result = matcher.match_response_headers(&ctx, &headers);
        assert!(result.remove_headers.is_empty());
    }

    #[test]
    fn cosmetic_rules_and_generichide() {
        let rules = parse_filter_list("example.com##.ad\nexample.com#@#.ad");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/index.html",
            req_host: "example.com",
            req_etld1: "example.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: false,
            request_type: RequestType::MAIN_FRAME,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_cosmetics(&ctx);
        assert!(result.css.is_empty());

        let rules = parse_filter_list("##.ad\n@@||example.com^$generichide");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let result = matcher.match_cosmetics(&ctx);
        assert!(result.css.is_empty());
        assert_eq!(result.enable_generic, false);
    }

    #[test]
    fn scriptlet_rules_and_exceptions() {
        let rules = parse_filter_list("example.com##+js(set-constant, foo, bar)");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/index.html",
            req_host: "example.com",
            req_etld1: "example.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: false,
            request_type: RequestType::MAIN_FRAME,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_cosmetics(&ctx);
        assert_eq!(result.scriptlets.len(), 1);
        assert_eq!(result.scriptlets[0].name, "set-constant");

        let rules = parse_filter_list(
            "example.com##+js(set-constant, foo, bar)\nexample.com#@#+js(set-constant, foo, bar)",
        );
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let result = matcher.match_cosmetics(&ctx);
        assert!(result.scriptlets.is_empty());

        let rules = parse_filter_list("#@#+js()");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let result = matcher.match_cosmetics(&ctx);
        assert!(result.scriptlets.is_empty());
    }

    #[test]
    fn important_blocks_ignore_exception() {
        let rules = parse_filter_list("||ads.com^$important\n@@||ads.com^");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://ads.com/script.js",
            req_host: "ads.com",
            req_etld1: "ads.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx);
        assert_eq!(result.decision, MatchDecision::Block);
    }

    #[test]
    fn redirect_rule_requires_block() {
        let rules = parse_filter_list("||example.com^$redirect-rule=noop.js");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/ad.js",
            req_host: "example.com",
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

        let result = matcher.match_request(&ctx);
        assert_eq!(result.decision, MatchDecision::Allow);
    }

    #[test]
    fn redirect_rule_exception_disables_redirect() {
        let rules = parse_filter_list(
            "||example.com^$redirect-rule=noop.js\n@@||example.com^$redirect-rule=noop.js\n||example.com^",
        );
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/ad.js",
            req_host: "example.com",
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

        let result = matcher.match_request(&ctx);
        assert_eq!(result.decision, MatchDecision::Block);
        assert!(result.redirect_url.is_none());
    }

    #[test]
    fn procedural_rules_respect_generichide_and_elemhide() {
        let rules = parse_filter_list("#?#.ad:has-text(foo)");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/index.html",
            req_host: "example.com",
            req_etld1: "example.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: false,
            request_type: RequestType::MAIN_FRAME,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_cosmetics(&ctx);
        assert_eq!(result.procedural.len(), 1);

        let rules = parse_filter_list("#?#.ad:has-text(foo)\n@@||example.com^$generichide");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let result = matcher.match_cosmetics(&ctx);
        assert!(result.procedural.is_empty());

        let rules = parse_filter_list("example.com#?#.ad:has-text(foo)\n@@||example.com^$elemhide");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let result = matcher.match_cosmetics(&ctx);
        assert!(result.procedural.is_empty());
    }

    #[test]
    fn badfilter_cancels_block_rule() {
        // Block rule with matching badfilter should be cancelled
        let mut rules = parse_filter_list("||ads.com^\n||ads.com^$badfilter");
        optimize_rules(&mut rules);
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://ads.com/script.js",
            req_host: "ads.com",
            req_etld1: "ads.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: true,
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
    fn badfilter_cancels_exception_rule() {
        // Exception rule with matching badfilter should be cancelled, allowing block
        let mut rules = parse_filter_list("||ads.com^\n@@||ads.com^\n@@||ads.com^$badfilter");
        optimize_rules(&mut rules);
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://ads.com/script.js",
            req_host: "ads.com",
            req_etld1: "ads.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx);
        assert_eq!(result.decision, MatchDecision::Block);
    }

    #[test]
    fn important_exception_beats_important_block() {
        // @@$important should beat $important block
        let rules = parse_filter_list("||ads.com^$important\n@@||ads.com^$important");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://ads.com/script.js",
            req_host: "ads.com",
            req_etld1: "ads.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: true,
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
    fn redirect_with_important_beats_exception() {
        // $redirect,important should redirect even with exception
        let rules = parse_filter_list("||ads.com^$redirect=noop.js,important\n@@||ads.com^");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://ads.com/script.js",
            req_host: "ads.com",
            req_etld1: "ads.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx);
        assert_eq!(result.decision, MatchDecision::Redirect);
        assert!(result.redirect_url.is_some());
    }

    #[test]
    fn specific_exception_beats_generic_block() {
        // Exception with domain constraint beats generic block for that domain
        let rules = parse_filter_list("||ads.com^\n@@||ads.com^$domain=safe.com");
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        // Request from safe.com - should be allowed
        let ctx_safe = RequestContext {
            url: "https://ads.com/script.js",
            req_host: "ads.com",
            req_etld1: "ads.com",
            site_host: "safe.com",
            site_etld1: "safe.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let result = matcher.match_request(&ctx_safe);
        assert_eq!(result.decision, MatchDecision::Allow);

        // Request from other.com - should be blocked
        let ctx_other = RequestContext {
            url: "https://ads.com/script.js",
            req_host: "ads.com",
            req_etld1: "ads.com",
            site_host: "other.com",
            site_etld1: "other.com",
            is_third_party: true,
            request_type: RequestType::SCRIPT,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "1",
        };

        let result = matcher.match_request(&ctx_other);
        assert_eq!(result.decision, MatchDecision::Block);
    }

    #[test]
    fn csp_multiple_rules_combine() {
        // Multiple CSP rules should all be applied
        let rules = parse_filter_list(
            "||example.com^$csp=script-src 'none'\n||example.com^$csp=frame-src 'self'",
        );
        let bytes = build_snapshot(&rules);
        let snapshot = Snapshot::load(&bytes).expect("snapshot should load");
        let matcher = Matcher::new(&snapshot);

        let ctx = RequestContext {
            url: "https://example.com/index.html",
            req_host: "example.com",
            req_etld1: "example.com",
            site_host: "example.com",
            site_etld1: "example.com",
            is_third_party: false,
            request_type: RequestType::MAIN_FRAME,
            scheme: SchemeMask::HTTPS,
            tab_id: 0,
            frame_id: 0,
            request_id: "0",
        };

        let headers = [ResponseHeader {
            name: "Content-Type",
            value: "text/html",
        }];

        let result = matcher.match_response_headers(&ctx, &headers);
        assert_eq!(result.csp_injections.len(), 2);
        assert!(result.csp_injections.contains(&"script-src 'none'".to_string()));
        assert!(result.csp_injections.contains(&"frame-src 'self'".to_string()));
    }
}
