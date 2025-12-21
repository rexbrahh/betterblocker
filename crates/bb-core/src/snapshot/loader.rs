//! Zero-copy UBX Snapshot Loader

use std::collections::HashMap;

use crate::hash::{Hash64, crc32};
use crate::psl::{load_psl_from_bytes, init_psl};
use super::format::*;

/// Error type for snapshot loading.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("Invalid magic bytes")]
    InvalidMagic,
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(u16),
    #[error("CRC32 mismatch: stored={stored}, computed={computed}")]
    Crc32Mismatch { stored: u32, computed: u32 },
    #[error("Invalid section: {0}")]
    InvalidSection(String),
    #[error("Data too short")]
    DataTooShort,
}

/// Section metadata.
#[derive(Debug, Clone)]
pub struct SectionInfo {
    pub id: SectionId,
    pub flags: u16,
    pub offset: usize,
    pub length: usize,
    pub uncompressed_length: usize,
    pub crc32: u32,
}

/// Zero-copy snapshot view.
pub struct Snapshot<'a> {
    data: &'a [u8],
    pub version: u16,
    pub flags: u16,
    pub build_id: u32,
    sections: HashMap<SectionId, SectionInfo>,
}

impl<'a> Snapshot<'a> {
    /// Load a snapshot from bytes.
    pub fn load(data: &'a [u8]) -> Result<Self, SnapshotError> {
        if data.len() < HEADER_SIZE {
            return Err(SnapshotError::DataTooShort);
        }

        // Validate magic
        if !validate_magic(data) {
            return Err(SnapshotError::InvalidMagic);
        }

        // Read header
        let version = read_u16_le(data, header::VERSION);
        if version != UBX_VERSION {
            return Err(SnapshotError::UnsupportedVersion(version));
        }

        let flags = read_u16_le(data, header::FLAGS);
        let section_count = read_u32_le(data, header::SECTION_COUNT) as usize;
        let section_dir_offset = read_u32_le(data, header::SECTION_DIR_OFFSET) as usize;
        let build_id = read_u32_le(data, header::BUILD_ID);

        // Validate CRC32 if present
        if flags & header_flags::HAS_CRC32 != 0 {
            let stored_crc = read_u32_le(data, header::SNAPSHOT_CRC32);
            
            // Compute CRC over everything except the CRC field
            let mut crc_data = Vec::with_capacity(data.len() - 4);
            crc_data.extend_from_slice(&data[..header::SNAPSHOT_CRC32]);
            crc_data.extend_from_slice(&data[header::SNAPSHOT_CRC32 + 4..]);
            let computed_crc = crc32(&crc_data);
            
            if stored_crc != computed_crc {
                return Err(SnapshotError::Crc32Mismatch {
                    stored: stored_crc,
                    computed: computed_crc,
                });
            }
        }

        // Parse section directory
        let mut sections = HashMap::new();
        for i in 0..section_count {
            let entry_offset = section_dir_offset + i * SECTION_ENTRY_SIZE;
            if entry_offset + SECTION_ENTRY_SIZE > data.len() {
                break;
            }

            let id_raw = read_u16_le(data, entry_offset + section_entry::ID);
            let id = match SectionId::try_from(id_raw) {
                Ok(id) => id,
                Err(_) => continue, // Skip unknown sections
            };

            let info = SectionInfo {
                id,
                flags: read_u16_le(data, entry_offset + section_entry::FLAGS),
                offset: read_u32_le(data, entry_offset + section_entry::OFFSET) as usize,
                length: read_u32_le(data, entry_offset + section_entry::LENGTH) as usize,
                uncompressed_length: read_u32_le(data, entry_offset + section_entry::UNCOMPRESSED_LENGTH) as usize,
                crc32: read_u32_le(data, entry_offset + section_entry::CRC32),
            };

            sections.insert(id, info);
        }

        let snapshot = Self {
            data,
            version,
            flags,
            build_id,
            sections,
        };

        // Initialize PSL if present
        if let Some(psl_section) = snapshot.sections.get(&SectionId::PslSets) {
            let psl_sets = load_psl_from_bytes(data, psl_section.offset);
            init_psl(psl_sets);
        }

        Ok(snapshot)
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    pub fn get_section(&self, id: SectionId) -> Option<&'a [u8]> {
        let info = self.sections.get(&id)?;
        if info.offset + info.length > self.data.len() {
            return None;
        }
        Some(&self.data[info.offset..info.offset + info.length])
    }

    /// Get section info.
    pub fn get_section_info(&self, id: SectionId) -> Option<&SectionInfo> {
        self.sections.get(&id)
    }

    /// Get string from string pool.
    pub fn get_string(&self, offset: usize, length: usize) -> Option<&'a str> {
        let section = self.get_section(SectionId::StrPool)?;
        if section.len() < 4 {
            return None;
        }
        
        // First 4 bytes are the pool length
        let pool_data = &section[4..];
        if offset + length > pool_data.len() {
            return None;
        }
        
        std::str::from_utf8(&pool_data[offset..offset + length]).ok()
    }

    /// Get domain block set view.
    pub fn domain_block_set(&self) -> DomainHashSet<'a> {
        self.get_section(SectionId::DomainSets)
            .map(|data| DomainHashSet::new(data, 0))
            .unwrap_or_else(|| DomainHashSet::empty())
    }

    /// Get domain allow set view.
    pub fn domain_allow_set(&self) -> DomainHashSet<'a> {
        self.get_section(SectionId::DomainSets)
            .map(|data| {
                // Allow set is after block set
                let block_capacity = read_u32_le(data, 0) as usize;
                let block_size = HASHMAP64_HEADER_SIZE + block_capacity * HASHMAP64_ENTRY_SIZE;
                if block_size < data.len() {
                    DomainHashSet::new(data, block_size)
                } else {
                    DomainHashSet::empty()
                }
            })
            .unwrap_or_else(|| DomainHashSet::empty())
    }

    pub fn domain_postings(&self) -> Option<&'a [u8]> {
        let data = self.get_section(SectionId::DomainSets)?;
        let block_capacity = read_u32_le(data, 0) as usize;
        let block_size = HASHMAP64_HEADER_SIZE + block_capacity * HASHMAP64_ENTRY_SIZE;
        if block_size + 4 > data.len() {
            return None;
        }

        let allow_capacity = read_u32_le(data, block_size) as usize;
        let allow_size = HASHMAP64_HEADER_SIZE + allow_capacity * HASHMAP64_ENTRY_SIZE;
        let postings_offset = block_size + allow_size;
        if postings_offset + 4 > data.len() {
            return None;
        }

        let len = read_u32_le(data, postings_offset) as usize;
        let start = postings_offset + 4;
        let available = data.len().saturating_sub(start);
        Some(&data[start..start + len.min(available)])
    }

    /// Get token dictionary view.
    pub fn token_dict(&self) -> TokenDict<'a> {
        self.get_section(SectionId::TokenDict)
            .map(|data| TokenDict::new(data))
            .unwrap_or_else(|| TokenDict::empty())
    }

    /// Get token postings data.
    pub fn token_postings(&self) -> &'a [u8] {
        self.get_section(SectionId::TokenPostings)
            .map(|data| {
                if data.len() < 4 {
                    &[]
                } else {
                    let len = read_u32_le(data, 0) as usize;
                    &data[4..4 + len.min(data.len() - 4)]
                }
            })
            .unwrap_or(&[])
    }

    /// Get pattern pool view.
    pub fn pattern_pool(&self) -> PatternPool<'a> {
        self.get_section(SectionId::PatternPool)
            .map(|data| PatternPool::new(data))
            .unwrap_or_else(|| PatternPool::empty())
    }

    /// Get rules view.
    pub fn rules(&self) -> RulesView<'a> {
        self.get_section(SectionId::Rules)
            .map(|data| RulesView::new(data))
            .unwrap_or_else(|| RulesView::empty())
    }

    /// Get domain constraints data.
    pub fn domain_constraints(&self) -> &'a [u8] {
        self.get_section(SectionId::DomainConstraintPool)
            .map(|data| {
                if data.len() < 4 {
                    &[]
                } else {
                    let len = read_u32_le(data, 0) as usize;
                    &data[4..4 + len.min(data.len() - 4)]
                }
            })
            .unwrap_or(&[])
    }
}

// =============================================================================
// Domain Hash Set (HashMap64toU32 view)
// =============================================================================

/// Zero-copy view into a domain hash map.
pub struct DomainHashSet<'a> {
    data: &'a [u8],
    offset: usize,
    capacity: usize,
}

impl<'a> DomainHashSet<'a> {
    fn new(data: &'a [u8], offset: usize) -> Self {
        let capacity = if offset + 4 <= data.len() {
            read_u32_le(data, offset) as usize
        } else {
            0
        };
        Self { data, offset, capacity }
    }

    fn empty() -> Self {
        Self { data: &[], offset: 0, capacity: 0 }
    }

    pub fn lookup(&self, hash: Hash64) -> Option<u32> {
        if self.capacity == 0 {
            return None;
        }

        let entries_offset = self.offset + HASHMAP64_HEADER_SIZE;
        let mask = self.capacity - 1; // Capacity is power of 2
        let mut idx = (hash.lo as usize) & mask;

        for _ in 0..self.capacity {
            let entry_offset = entries_offset + idx * HASHMAP64_ENTRY_SIZE;
            if entry_offset + 12 > self.data.len() {
                return None;
            }

            let lo = read_u32_le(self.data, entry_offset);
            let hi = read_u32_le(self.data, entry_offset + 4);

            // Empty slot
            if lo == 0 && hi == 0 {
                return None;
            }

            // Match found
            if lo == hash.lo && hi == hash.hi {
                return Some(read_u32_le(self.data, entry_offset + 8));
            }

            // Linear probing
            idx = (idx + 1) & mask;
        }

        None
    }

    /// Check if a domain hash exists.
    pub fn contains(&self, hash: Hash64) -> bool {
        self.lookup(hash).is_some()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn entry_count(&self) -> usize {
        if self.capacity == 0 || self.offset + HASHMAP64_HEADER_SIZE > self.data.len() {
            return 0;
        }
        read_u32_le(self.data, self.offset + 4) as usize
    }
}

// =============================================================================
// Token Dictionary View
// =============================================================================

/// Token dictionary entry.
#[derive(Debug, Clone, Copy)]
pub struct TokenEntry {
    pub token_hash: u32,
    pub postings_offset: usize,
    pub rule_count: usize,
}

/// Zero-copy view into token dictionary.
pub struct TokenDict<'a> {
    data: &'a [u8],
    capacity: usize,
}

impl<'a> TokenDict<'a> {
    fn new(data: &'a [u8]) -> Self {
        let capacity = if data.len() >= 4 {
            read_u32_le(data, 0) as usize
        } else {
            0
        };
        Self { data, capacity }
    }

    fn empty() -> Self {
        Self { data: &[], capacity: 0 }
    }

    /// Look up a token hash.
    pub fn lookup(&self, token_hash: u32) -> Option<TokenEntry> {
        if self.capacity == 0 {
            return None;
        }

        let entries_offset = TOKEN_DICT_HEADER_SIZE;
        let mask = self.capacity - 1;
        let mut idx = (token_hash as usize) & mask;

        for _ in 0..self.capacity {
            let entry_offset = entries_offset + idx * TOKEN_DICT_ENTRY_SIZE;
            if entry_offset + 12 > self.data.len() {
                return None;
            }

            let stored_hash = read_u32_le(self.data, entry_offset);

            // Empty slot
            if stored_hash == 0 {
                return None;
            }

            // Match
            if stored_hash == token_hash {
                return Some(TokenEntry {
                    token_hash: stored_hash,
                    postings_offset: read_u32_le(self.data, entry_offset + 4) as usize,
                    rule_count: read_u32_le(self.data, entry_offset + 8) as usize,
                });
            }

            idx = (idx + 1) & mask;
        }

        None
    }
}

// =============================================================================
// Pattern Pool View
// =============================================================================

/// Pattern entry.
#[derive(Debug, Clone, Copy)]
pub struct PatternEntry {
    pub prog_offset: usize,
    pub prog_len: usize,
    pub anchor_type: u8,
    pub flags: u8,
    pub host_hash_lo: u32,
    pub host_hash_hi: u32,
}

/// Zero-copy view into pattern pool.
pub struct PatternPool<'a> {
    data: &'a [u8],
    pattern_count: usize,
    prog_bytes_offset: usize,
}

impl<'a> PatternPool<'a> {
    fn new(data: &'a [u8]) -> Self {
        if data.len() < 4 {
            return Self::empty();
        }

        let pattern_count = read_u32_le(data, 0) as usize;
        let index_size = pattern_count * PATTERN_INDEX_ENTRY_SIZE;
        let prog_bytes_offset = 4 + index_size + 4; // +4 for prog_bytes_len

        Self { data, pattern_count, prog_bytes_offset }
    }

    fn empty() -> Self {
        Self { data: &[], pattern_count: 0, prog_bytes_offset: 0 }
    }

    /// Get a pattern entry by ID.
    pub fn get_pattern(&self, pattern_id: usize) -> Option<PatternEntry> {
        if pattern_id >= self.pattern_count {
            return None;
        }

        let entry_offset = 4 + pattern_id * PATTERN_INDEX_ENTRY_SIZE;
        if entry_offset + PATTERN_INDEX_ENTRY_SIZE > self.data.len() {
            return None;
        }

        Some(PatternEntry {
            prog_offset: read_u32_le(self.data, entry_offset + pattern_entry::PROG_OFF) as usize,
            prog_len: read_u16_le(self.data, entry_offset + pattern_entry::PROG_LEN) as usize,
            anchor_type: self.data[entry_offset + pattern_entry::ANCHOR_TYPE],
            flags: self.data[entry_offset + pattern_entry::FLAGS],
            host_hash_lo: read_u32_le(self.data, entry_offset + pattern_entry::HOST_HASH_LO),
            host_hash_hi: read_u32_le(self.data, entry_offset + pattern_entry::HOST_HASH_HI),
        })
    }

    /// Get program bytes for a pattern.
    pub fn get_program(&self, entry: &PatternEntry) -> &'a [u8] {
        let start = self.prog_bytes_offset + entry.prog_offset;
        let end = start + entry.prog_len;
        if end <= self.data.len() {
            &self.data[start..end]
        } else {
            &[]
        }
    }
}

// =============================================================================
// Rules View (SoA layout)
// =============================================================================

/// Zero-copy view into rules table.
pub struct RulesView<'a> {
    data: &'a [u8],
    pub count: usize,
    // Precomputed offsets for each array
    action_offset: usize,
    flags_offset: usize,
    type_mask_offset: usize,
    party_mask_offset: usize,
    scheme_mask_offset: usize,
    pattern_id_offset: usize,
    domain_constraint_offset: usize,
    option_id_offset: usize,
    priority_offset: usize,
    list_id_offset: usize,
}

impl<'a> RulesView<'a> {
    fn new(data: &'a [u8]) -> Self {
        if data.len() < 4 {
            return Self::empty();
        }

        let count = read_u32_le(data, 0) as usize;
        let mut offset = 4;

        // Calculate offsets for each array
        let action_offset = offset;
        offset = align_offset(offset + count, 2);

        let flags_offset = offset;
        offset = align_offset(offset + count * 2, 4);

        let type_mask_offset = offset;
        offset += count * 4;

        let party_mask_offset = offset;
        offset = align_offset(offset + count, 1);

        let scheme_mask_offset = offset;
        offset = align_offset(offset + count, 4);

        let pattern_id_offset = offset;
        offset += count * 4;

        let domain_constraint_offset = offset;
        offset += count * 4;

        let option_id_offset = offset;
        offset += count * 4;

        let priority_offset = offset;
        offset = align_offset(offset + count * 2, 2);

        let list_id_offset = offset;

        Self {
            data,
            count,
            action_offset,
            flags_offset,
            type_mask_offset,
            party_mask_offset,
            scheme_mask_offset,
            pattern_id_offset,
            domain_constraint_offset,
            option_id_offset,
            priority_offset,
            list_id_offset,
        }
    }

    fn empty() -> Self {
        Self {
            data: &[],
            count: 0,
            action_offset: 0,
            flags_offset: 0,
            type_mask_offset: 0,
            party_mask_offset: 0,
            scheme_mask_offset: 0,
            pattern_id_offset: 0,
            domain_constraint_offset: 0,
            option_id_offset: 0,
            priority_offset: 0,
            list_id_offset: 0,
        }
    }

    pub fn action(&self, rule_id: usize) -> u8 {
        if rule_id >= self.count { return 0; }
        self.data.get(self.action_offset + rule_id).copied().unwrap_or(0)
    }

    pub fn flags(&self, rule_id: usize) -> u16 {
        if rule_id >= self.count { return 0; }
        let offset = self.flags_offset + rule_id * 2;
        read_u16_le(self.data, offset)
    }

    pub fn type_mask(&self, rule_id: usize) -> u32 {
        if rule_id >= self.count { return 0; }
        let offset = self.type_mask_offset + rule_id * 4;
        read_u32_le(self.data, offset)
    }

    pub fn party_mask(&self, rule_id: usize) -> u8 {
        if rule_id >= self.count { return 0; }
        self.data.get(self.party_mask_offset + rule_id).copied().unwrap_or(0)
    }

    pub fn scheme_mask(&self, rule_id: usize) -> u8 {
        if rule_id >= self.count { return 0; }
        self.data.get(self.scheme_mask_offset + rule_id).copied().unwrap_or(0)
    }

    pub fn pattern_id(&self, rule_id: usize) -> u32 {
        if rule_id >= self.count { return NO_PATTERN; }
        let offset = self.pattern_id_offset + rule_id * 4;
        read_u32_le(self.data, offset)
    }

    pub fn domain_constraint_offset(&self, rule_id: usize) -> u32 {
        if rule_id >= self.count { return NO_CONSTRAINT; }
        let offset = self.domain_constraint_offset + rule_id * 4;
        read_u32_le(self.data, offset)
    }

    pub fn option_id(&self, rule_id: usize) -> u32 {
        if rule_id >= self.count { return 0; }
        let offset = self.option_id_offset + rule_id * 4;
        read_u32_le(self.data, offset)
    }

    pub fn priority(&self, rule_id: usize) -> i16 {
        if rule_id >= self.count { return 0; }
        let offset = self.priority_offset + rule_id * 2;
        read_i16_le(self.data, offset)
    }

    pub fn list_id(&self, rule_id: usize) -> u16 {
        if rule_id >= self.count { return 0; }
        let offset = self.list_id_offset + rule_id * 2;
        read_u16_le(self.data, offset)
    }

    pub fn has_pattern(&self, rule_id: usize) -> bool {
        self.pattern_id(rule_id) != NO_PATTERN
    }

    pub fn has_constraints(&self, rule_id: usize) -> bool {
        self.domain_constraint_offset(rule_id) != NO_CONSTRAINT
    }
}

// =============================================================================
// Varint Decoder
// =============================================================================

/// Decode a single unsigned LEB128 varint.
/// Returns (value, bytes_read).
pub fn decode_varint(data: &[u8], offset: usize) -> (u32, usize) {
    let mut result: u32 = 0;
    let mut shift = 0;
    let mut bytes_read = 0;

    while offset + bytes_read < data.len() {
        let byte = data[offset + bytes_read];
        bytes_read += 1;

        result |= ((byte & 0x7f) as u32) << shift;

        if byte & 0x80 == 0 {
            break;
        }

        shift += 7;
        if shift > 35 {
            break; // Overflow protection
        }
    }

    (result, bytes_read)
}

/// Decode a delta-encoded posting list.
pub fn decode_posting_list(data: &[u8], offset: usize, count: usize) -> Vec<u32> {
    let mut result = Vec::with_capacity(count);
    let mut pos = offset;
    let mut prev_id: u32 = 0;

    for _ in 0..count {
        if pos >= data.len() {
            break;
        }
        let (delta, bytes_read) = decode_varint(data, pos);
        pos += bytes_read;
        prev_id = prev_id.wrapping_add(delta);
        result.push(prev_id);
    }

    result
}

pub fn decode_posting_list_with_count(data: &[u8], offset: usize) -> Vec<u32> {
    if offset + 4 > data.len() {
        return Vec::new();
    }
    let count = read_u32_le(data, offset) as usize;
    decode_posting_list(data, offset + 4, count)
}
