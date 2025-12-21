/**
 * UBX Snapshot Format v1 Constants and Layouts
 * 
 * This file defines all binary format constants for the UBX snapshot.
 * All values are little-endian.
 */

// =============================================================================
// Magic and Version
// =============================================================================

/** Magic bytes: "UBX1" */
export const UBX_MAGIC = new Uint8Array([0x55, 0x42, 0x58, 0x31]); // "UBX1"

/** Current format version */
export const UBX_VERSION = 1;

// =============================================================================
// Header Layout (64 bytes fixed)
// =============================================================================

export const HEADER_SIZE = 64;

/** Header field offsets */
export const Header = {
  /** u8[4] magic = "UBX1" */
  MAGIC: 0,
  /** u16 version */
  VERSION: 4,
  /** u16 flags (bit0: hasCrc32) */
  FLAGS: 6,
  /** u32 headerBytes (always 64) */
  HEADER_BYTES: 8,
  /** u32 sectionCount */
  SECTION_COUNT: 12,
  /** u32 sectionDirOffset */
  SECTION_DIR_OFFSET: 16,
  /** u32 sectionDirBytes */
  SECTION_DIR_BYTES: 20,
  /** u32 buildId */
  BUILD_ID: 24,
  /** u32 snapshotCrc32 (if flags.hasCrc32) */
  SNAPSHOT_CRC32: 28,
  // Reserved: 32-60
} as const;

/** Header flags */
export const HeaderFlags = {
  /** Snapshot includes CRC32 checksum */
  HAS_CRC32: 1 << 0,
} as const;

// =============================================================================
// Section Directory Entry Layout (24 bytes each)
// =============================================================================

export const SECTION_ENTRY_SIZE = 24;

/** Section entry field offsets */
export const SectionEntry = {
  /** u16 section id */
  ID: 0,
  /** u16 flags (compression, etc.) */
  FLAGS: 2,
  /** u32 file offset */
  OFFSET: 4,
  /** u32 byte length in file */
  LENGTH: 8,
  /** u32 uncompressed length (0 if not compressed) */
  UNCOMPRESSED_LENGTH: 12,
  /** u32 CRC32 (0 if unused) */
  CRC32: 16,
  // Reserved: 20-24
} as const;

// =============================================================================
// Section IDs
// =============================================================================

export const SectionId = {
  /** String pool for all interned strings */
  STRPOOL: 0x0001,
  /** Public Suffix List hash sets */
  PSL_SETS: 0x0002,
  /** Domain hash sets for host-only rules */
  DOMAIN_SETS: 0x0003,
  /** Token dictionary for URL pattern matching */
  TOKEN_DICT: 0x0004,
  /** Token posting lists (varint-encoded rule IDs) */
  TOKEN_POSTINGS: 0x0005,
  /** Compiled pattern programs */
  PATTERN_POOL: 0x0006,
  /** Main rules table (SoA layout) */
  RULES: 0x0007,
  /** Domain constraint data for $domain= */
  DOMAIN_CONSTRAINT_POOL: 0x0008,
  /** Redirect resource mappings */
  REDIRECT_RESOURCES: 0x0009,
  /** removeparam specifications */
  REMOVEPARAM_SPECS: 0x000a,
  /** CSP injection specifications */
  CSP_SPECS: 0x000b,
  /** Header matching specifications */
  HEADER_SPECS: 0x000c,
  /** Response header removal rules */
  RESPONSEHEADER_RULES: 0x000d,
  /** Cosmetic filter rules */
  COSMETIC_RULES: 0x000e,
  /** Procedural cosmetic rules */
  PROCEDURAL_RULES: 0x000f,
  /** Scriptlet injection rules */
  SCRIPTLET_RULES: 0x0010,
} as const;

export type SectionIdType = typeof SectionId[keyof typeof SectionId];

// =============================================================================
// HashSet64 Layout (for domain/PSL hash sets)
// =============================================================================

export const HASHSET64_HEADER_SIZE = 20;

/** HashSet64 header offsets */
export const HashSet64Header = {
  /** u32 capacity (power of 2) */
  CAPACITY: 0,
  /** u32 count (number of entries) */
  COUNT: 4,
  /** u32 seedLo */
  SEED_LO: 8,
  /** u32 seedHi */
  SEED_HI: 12,
  /** u32 reserved */
  RESERVED: 16,
} as const;

/** Each entry is (lo: u32, hi: u32) = 8 bytes */
export const HASHSET64_ENTRY_SIZE = 8;

// =============================================================================
// HashMap64toU32 Layout (for domain -> ruleId mapping)
// =============================================================================

export const HASHMAP64_HEADER_SIZE = 20;

/** HashMap64toU32 header offsets */
export const HashMap64Header = {
  /** u32 capacity (power of 2) */
  CAPACITY: 0,
  /** u32 count */
  COUNT: 4,
  /** u32 seedLo */
  SEED_LO: 8,
  /** u32 seedHi */
  SEED_HI: 12,
  /** u32 reserved */
  RESERVED: 16,
} as const;

/** Each entry is (lo: u32, hi: u32, value: u32) = 12 bytes */
export const HASHMAP64_ENTRY_SIZE = 12;

// =============================================================================
// TOKEN_DICT Layout
// =============================================================================

export const TOKEN_DICT_HEADER_SIZE = 16;

/** Token dictionary header */
export const TokenDictHeader = {
  /** u32 capacity */
  CAPACITY: 0,
  /** u32 count */
  COUNT: 4,
  /** u32 seed */
  SEED: 8,
  /** u32 reserved */
  RESERVED: 12,
} as const;

/** Token dict entry: (tokenHash: u32, postingsOff: u32, ruleCount: u32) = 12 bytes */
export const TOKEN_DICT_ENTRY_SIZE = 12;

export const TokenDictEntry = {
  TOKEN_HASH: 0,
  POSTINGS_OFF: 4,
  RULE_COUNT: 8,
} as const;

// =============================================================================
// PATTERN_POOL Layout
// =============================================================================

export const PATTERN_INDEX_ENTRY_SIZE = 24;

/** Pattern index entry offsets */
export const PatternIndexEntry = {
  /** u32 program offset into progBytes */
  PROG_OFF: 0,
  /** u16 program length */
  PROG_LEN: 4,
  /** u8 anchor type (0=none, 1=left, 2=hostname, 3=regex) */
  ANCHOR_TYPE: 6,
  /** u8 flags (bit0=caseSensitive, bit1=hasRightAnchor, bit2=hasBoundaryCaret) */
  FLAGS: 7,
  /** u32 hostHashLo (0 if none) */
  HOST_HASH_LO: 8,
  /** u32 hostHashHi (0 if none) */
  HOST_HASH_HI: 12,
  /** u32 reserved */
  RESERVED: 16,
  /** u32 reserved2 */
  RESERVED2: 20,
} as const;

/** Pattern anchor types */
export const PatternAnchorType = {
  NONE: 0,
  LEFT: 1,      // |pattern
  HOSTNAME: 2,  // ||pattern
  REGEX: 3,     // /regex/
} as const;

/** Pattern flags */
export const PatternFlags = {
  CASE_SENSITIVE: 1 << 0,
  HAS_RIGHT_ANCHOR: 1 << 1,
  HAS_BOUNDARY_CARET: 1 << 2,
} as const;

// =============================================================================
// Pattern Bytecode Opcodes
// =============================================================================

export const PatternOp = {
  /** FIND_LIT <strOff:u32> <strLen:u16> - find literal substring */
  FIND_LIT: 0x01,
  /** ASSERT_START - current index must be 0 */
  ASSERT_START: 0x02,
  /** ASSERT_END - current index must be at end */
  ASSERT_END: 0x03,
  /** ASSERT_BOUNDARY - next char must be separator (ABP ^) */
  ASSERT_BOUNDARY: 0x04,
  /** SKIP_ANY - wildcard (*), no constraint on position */
  SKIP_ANY: 0x05,
  /** HOST_ANCHOR - verify host hash matches */
  HOST_ANCHOR: 0x06,
  /** DONE - pattern match complete */
  DONE: 0x07,
} as const;

// =============================================================================
// RULES Section Layout (SoA arrays)
// =============================================================================

/** Rules section has u32 ruleCount followed by aligned arrays */
export const RulesArrays = {
  /** u8[] action per rule */
  ACTION: 0,
  /** u16[] flags per rule */
  FLAGS: 1,
  /** u32[] typeMask per rule */
  TYPE_MASK: 2,
  /** u8[] partyMask per rule */
  PARTY_MASK: 3,
  /** u8[] schemeMask per rule */
  SCHEME_MASK: 4,
  /** u32[] patternId per rule (0xFFFFFFFF if none) */
  PATTERN_ID: 5,
  /** u32[] domainConstraintOff per rule (0xFFFFFFFF if none) */
  DOMAIN_CONSTRAINT_OFF: 6,
  /** u32[] optionId per rule (meaning depends on action) */
  OPTION_ID: 7,
  /** i16[] priority per rule (for redirects) */
  PRIORITY: 8,
  /** u16[] listId per rule */
  LIST_ID: 9,
  /** u32[] rawTextOff per rule (into STRPOOL) */
  RAW_TEXT_OFF: 10,
  /** u32[] rawTextLen per rule */
  RAW_TEXT_LEN: 11,
} as const;

/** No pattern/constraint sentinel */
export const NO_PATTERN = 0xffffffff;
export const NO_CONSTRAINT = 0xffffffff;

// =============================================================================
// DOMAIN_CONSTRAINT_POOL Record Layout
// =============================================================================

/** Domain constraint record header */
export const DomainConstraint = {
  /** u16 include count */
  INCLUDE_COUNT: 0,
  /** u16 exclude count */
  EXCLUDE_COUNT: 2,
  // Followed by: includeCount * (lo u32, hi u32)
  // Followed by: excludeCount * (lo u32, hi u32)
} as const;

// =============================================================================
// REDIRECT_RESOURCES Layout
// =============================================================================

export const REDIRECT_RESOURCE_ENTRY_SIZE = 20;

export const RedirectResourceEntry = {
  /** u32 token string offset (into STRPOOL) */
  TOKEN_STR_OFF: 0,
  /** u32 token string length */
  TOKEN_STR_LEN: 4,
  /** u32 path string offset (into STRPOOL) */
  PATH_STR_OFF: 8,
  /** u32 path string length */
  PATH_STR_LEN: 12,
  /** u8 mime kind (0=js, 1=css, 2=img, 3=txt, etc.) */
  MIME_KIND: 16,
  // 3 bytes reserved
} as const;

export const MimeKind = {
  JAVASCRIPT: 0,
  CSS: 1,
  IMAGE: 2,
  TEXT: 3,
  JSON: 4,
  XML: 5,
  HTML: 6,
  OTHER: 255,
} as const;

// =============================================================================
// REMOVEPARAM_SPECS Layout
// =============================================================================

export const REMOVEPARAM_SPEC_SIZE = 24;

export const RemoveparamSpec = {
  /** u8 mode (0=removeAll, 1=literalName, 2=regexNameEqValue) */
  MODE: 0,
  // 3 bytes reserved
  /** u32 string offset (literal or regex) */
  STR_OFF: 4,
  /** u32 string length */
  STR_LEN: 8,
  // 12 bytes reserved
} as const;

export const RemoveparamMode = {
  REMOVE_ALL: 0,
  LITERAL_NAME: 1,
  REGEX_NAME_VALUE: 2,
} as const;

// =============================================================================
// CSP_SPECS Layout
// =============================================================================

export const CSP_SPEC_SIZE = 8;

export const CspSpec = {
  /** u32 CSP directive string offset (into STRPOOL) */
  CSP_STR_OFF: 0,
  /** u32 CSP directive string length */
  CSP_STR_LEN: 4,
} as const;

// =============================================================================
// HEADER_SPECS Layout
// =============================================================================

export const HEADER_SPEC_SIZE = 24;

export const HeaderSpec = {
  /** u32 header name offset */
  HEADER_NAME_OFF: 0,
  /** u32 header name length */
  HEADER_NAME_LEN: 4,
  /** u8 match kind (0=presenceOnly, 1=literal, 2=regex) */
  MATCH_KIND: 8,
  /** u8 invert (1 if ~ prefix) */
  INVERT: 9,
  // 2 bytes reserved
  /** u32 value offset (literal or regex) */
  VALUE_OFF: 12,
  /** u32 value length */
  VALUE_LEN: 16,
  // 4 bytes reserved
} as const;

export const HeaderMatchKind = {
  PRESENCE_ONLY: 0,
  LITERAL: 1,
  REGEX: 2,
} as const;

// =============================================================================
// RESPONSEHEADER_RULES Layout
// =============================================================================

/** Allowed response headers that can be removed (uBO safety constraints) */
export const ResponseHeaderId = {
  LOCATION: 0,
  REFRESH: 1,
  REPORT_TO: 2,
  SET_COOKIE: 3,
} as const;

// =============================================================================
// COSMETIC_RULES Layout
// =============================================================================

export const CosmeticRecordHeader = {
  /** u16 hide count */
  HIDE_COUNT: 0,
  /** u16 exception count */
  EXCEPTION_COUNT: 2,
  // Followed by: hideCount * (strOff u32, strLen u32)
  // Followed by: exceptionCount * (strOff u32, strLen u32)
  // Followed by: u32 flags (bit0=disableAllCosmetics, bit1=disableGenericCosmetics)
} as const;

export const CosmeticFlags = {
  DISABLE_ALL_COSMETICS: 1 << 0,
  DISABLE_GENERIC_COSMETICS: 1 << 1,
} as const;

// =============================================================================
// Helpers
// =============================================================================

/**
 * Align an offset to a given boundary (power of 2).
 */
export function alignOffset(offset: number, alignment: number): number {
  return (offset + alignment - 1) & ~(alignment - 1);
}

/**
 * Validate magic bytes.
 */
export function validateMagic(data: Uint8Array): boolean {
  if (data.length < 4) return false;
  return (
    data[0] === UBX_MAGIC[0] &&
    data[1] === UBX_MAGIC[1] &&
    data[2] === UBX_MAGIC[2] &&
    data[3] === UBX_MAGIC[3]
  );
}
