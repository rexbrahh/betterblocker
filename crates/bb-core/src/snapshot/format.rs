//! UBX Snapshot Format v1 Constants
//!
//! All values are little-endian.

/// Magic bytes: "UBX1"
pub const UBX_MAGIC: [u8; 4] = [0x55, 0x42, 0x58, 0x31];

/// Current format version
pub const UBX_VERSION: u16 = 1;

/// Header size in bytes
pub const HEADER_SIZE: usize = 64;

/// Section directory entry size
pub const SECTION_ENTRY_SIZE: usize = 24;

// =============================================================================
// Header Field Offsets
// =============================================================================

/// Header field byte offsets.
pub mod header {
    /// u8[4] magic = "UBX1"
    pub const MAGIC: usize = 0;
    /// u16 version
    pub const VERSION: usize = 4;
    /// u16 flags
    pub const FLAGS: usize = 6;
    /// u32 headerBytes (always 64)
    pub const HEADER_BYTES: usize = 8;
    /// u32 sectionCount
    pub const SECTION_COUNT: usize = 12;
    /// u32 sectionDirOffset
    pub const SECTION_DIR_OFFSET: usize = 16;
    /// u32 sectionDirBytes
    pub const SECTION_DIR_BYTES: usize = 20;
    /// u32 buildId
    pub const BUILD_ID: usize = 24;
    /// u32 snapshotCrc32
    pub const SNAPSHOT_CRC32: usize = 28;
}

/// Header flags.
pub mod header_flags {
    /// Snapshot includes CRC32 checksum
    pub const HAS_CRC32: u16 = 1 << 0;
}

// =============================================================================
// Section Directory Entry Offsets
// =============================================================================

pub mod section_entry {
    /// u16 section id
    pub const ID: usize = 0;
    /// u16 flags
    pub const FLAGS: usize = 2;
    /// u32 file offset
    pub const OFFSET: usize = 4;
    /// u32 byte length
    pub const LENGTH: usize = 8;
    /// u32 uncompressed length (0 if not compressed)
    pub const UNCOMPRESSED_LENGTH: usize = 12;
    /// u32 CRC32 (0 if unused)
    pub const CRC32: usize = 16;
}

// =============================================================================
// Section IDs
// =============================================================================

/// Section type identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum SectionId {
    /// String pool for all interned strings
    StrPool = 0x0001,
    /// Public Suffix List hash sets
    PslSets = 0x0002,
    /// Domain hash sets for host-only rules
    DomainSets = 0x0003,
    /// Token dictionary for URL pattern matching
    TokenDict = 0x0004,
    /// Token posting lists
    TokenPostings = 0x0005,
    /// Compiled pattern programs
    PatternPool = 0x0006,
    /// Main rules table
    Rules = 0x0007,
    /// Domain constraint data
    DomainConstraintPool = 0x0008,
    /// Redirect resource mappings
    RedirectResources = 0x0009,
    /// removeparam specifications
    RemoveparamSpecs = 0x000A,
    /// CSP injection specifications
    CspSpecs = 0x000B,
    /// Header matching specifications
    HeaderSpecs = 0x000C,
    /// Response header removal rules
    ResponseHeaderRules = 0x000D,
    /// Cosmetic filter rules
    CosmeticRules = 0x000E,
    /// Procedural cosmetic rules
    ProceduralRules = 0x000F,
    /// Scriptlet injection rules
    ScriptletRules = 0x0010,
}

impl TryFrom<u16> for SectionId {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0001 => Ok(Self::StrPool),
            0x0002 => Ok(Self::PslSets),
            0x0003 => Ok(Self::DomainSets),
            0x0004 => Ok(Self::TokenDict),
            0x0005 => Ok(Self::TokenPostings),
            0x0006 => Ok(Self::PatternPool),
            0x0007 => Ok(Self::Rules),
            0x0008 => Ok(Self::DomainConstraintPool),
            0x0009 => Ok(Self::RedirectResources),
            0x000A => Ok(Self::RemoveparamSpecs),
            0x000B => Ok(Self::CspSpecs),
            0x000C => Ok(Self::HeaderSpecs),
            0x000D => Ok(Self::ResponseHeaderRules),
            0x000E => Ok(Self::CosmeticRules),
            0x000F => Ok(Self::ProceduralRules),
            0x0010 => Ok(Self::ScriptletRules),
            _ => Err(()),
        }
    }
}

// =============================================================================
// HashSet64 / HashMap64 Layout
// =============================================================================

/// HashSet64 header size
pub const HASHSET64_HEADER_SIZE: usize = 20;

/// HashSet64 entry size (lo, hi)
pub const HASHSET64_ENTRY_SIZE: usize = 8;

/// HashMap64toU32 header size
pub const HASHMAP64_HEADER_SIZE: usize = 20;

/// HashMap64toU32 entry size (lo, hi, value)
pub const HASHMAP64_ENTRY_SIZE: usize = 12;

// =============================================================================
// Token Dictionary Layout
// =============================================================================

/// Token dictionary header size
pub const TOKEN_DICT_HEADER_SIZE: usize = 16;

/// Token dictionary entry size
pub const TOKEN_DICT_ENTRY_SIZE: usize = 12;

pub mod token_dict_entry {
    pub const TOKEN_HASH: usize = 0;
    pub const POSTINGS_OFF: usize = 4;
    pub const RULE_COUNT: usize = 8;
}

// =============================================================================
// Pattern Pool Layout
// =============================================================================

/// Pattern index entry size
pub const PATTERN_INDEX_ENTRY_SIZE: usize = 24;

pub mod pattern_entry {
    pub const PROG_OFF: usize = 0;
    pub const PROG_LEN: usize = 4;
    pub const ANCHOR_TYPE: usize = 6;
    pub const FLAGS: usize = 7;
    pub const HOST_HASH_LO: usize = 8;
    pub const HOST_HASH_HI: usize = 12;
}

/// Pattern anchor types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PatternAnchorType {
    None = 0,
    Left = 1,      // |pattern
    Hostname = 2,  // ||pattern
    Regex = 3,     // /regex/
}

// =============================================================================
// Pattern Bytecode Opcodes
// =============================================================================

/// Pattern bytecode opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PatternOp {
    /// Find literal substring
    FindLit = 0x01,
    /// Assert at start
    AssertStart = 0x02,
    /// Assert at end
    AssertEnd = 0x03,
    /// Assert boundary (^)
    AssertBoundary = 0x04,
    /// Skip any (*)
    SkipAny = 0x05,
    /// Host anchor (||)
    HostAnchor = 0x06,
    /// Done
    Done = 0x07,
}

impl TryFrom<u8> for PatternOp {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::FindLit),
            0x02 => Ok(Self::AssertStart),
            0x03 => Ok(Self::AssertEnd),
            0x04 => Ok(Self::AssertBoundary),
            0x05 => Ok(Self::SkipAny),
            0x06 => Ok(Self::HostAnchor),
            0x07 => Ok(Self::Done),
            _ => Err(()),
        }
    }
}

// =============================================================================
// Sentinels
// =============================================================================

/// No pattern sentinel
pub const NO_PATTERN: u32 = 0xFFFF_FFFF;

/// No constraint sentinel
pub const NO_CONSTRAINT: u32 = 0xFFFF_FFFF;

// =============================================================================
// Helpers
// =============================================================================

/// Align offset to boundary.
#[inline]
pub const fn align_offset(offset: usize, alignment: usize) -> usize {
    (offset + alignment - 1) & !(alignment - 1)
}

/// Validate magic bytes.
#[inline]
pub fn validate_magic(data: &[u8]) -> bool {
    data.len() >= 4 && data[..4] == UBX_MAGIC
}

/// Read u16 little-endian.
#[inline]
pub fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

/// Read u32 little-endian.
#[inline]
pub fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

/// Read i16 little-endian.
#[inline]
pub fn read_i16_le(data: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([data[offset], data[offset + 1]])
}
