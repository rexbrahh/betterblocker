/**
 * Zero-copy UBX Snapshot Loader
 * 
 * Loads a compiled UBX snapshot into memory-mapped typed array views.
 * No parsing or object creation - just validation and view setup.
 */

import type { Hash64 } from '../types.js';
import { crc32 } from '../hash.js';
import { loadPSLFromSnapshot, initPSL } from '../psl.js';
import {
  UBX_VERSION,
  Header,
  HeaderFlags,
  SECTION_ENTRY_SIZE,
  SectionEntry,
  SectionId,
  HASHMAP64_HEADER_SIZE,
  HASHMAP64_ENTRY_SIZE,
  TOKEN_DICT_HEADER_SIZE,
  TOKEN_DICT_ENTRY_SIZE,
  TokenDictEntry,
  PATTERN_INDEX_ENTRY_SIZE,
  PatternIndexEntry,
  NO_PATTERN,
  NO_CONSTRAINT,
  validateMagic,
  alignOffset,
  type SectionIdType,
} from './format.js';

// =============================================================================
// Snapshot Section Info
// =============================================================================

export interface SectionInfo {
  id: SectionIdType;
  flags: number;
  offset: number;
  length: number;
  uncompressedLength: number;
  crc32: number;
}

// =============================================================================
// Snapshot Class
// =============================================================================

export class Snapshot {
  /** Raw buffer containing the snapshot */
  readonly buffer: ArrayBuffer;
  /** DataView for reading header and mixed fields */
  readonly view: DataView;
  /** Byte view of entire buffer */
  readonly bytes: Uint8Array;
  
  /** Snapshot format version */
  readonly version: number;
  /** Header flags */
  readonly flags: number;
  /** Build ID */
  readonly buildId: number;
  
  /** Section directory (id -> SectionInfo) */
  readonly sections: Map<SectionIdType, SectionInfo>;
  
  // Cached section views
  private _strPool: Uint8Array | null = null;
  private _domainBlockSet: DomainHashSet | null = null;
  private _domainAllowSet: DomainHashSet | null = null;
  private _tokenDict: TokenDict | null = null;
  private _tokenPostings: Uint8Array | null = null;
  private _patternPool: PatternPool | null = null;
  private _rules: RulesView | null = null;
  private _domainConstraints: Uint8Array | null = null;
  
  private constructor(buffer: ArrayBuffer) {
    this.buffer = buffer;
    this.view = new DataView(buffer);
    this.bytes = new Uint8Array(buffer);
    
    // Read header
    this.version = this.view.getUint16(Header.VERSION, true);
    this.flags = this.view.getUint16(Header.FLAGS, true);
    this.buildId = this.view.getUint32(Header.BUILD_ID, true);
    
    // Parse section directory
    this.sections = new Map();
    const sectionCount = this.view.getUint32(Header.SECTION_COUNT, true);
    const sectionDirOffset = this.view.getUint32(Header.SECTION_DIR_OFFSET, true);
    
    for (let i = 0; i < sectionCount; i++) {
      const entryOffset = sectionDirOffset + i * SECTION_ENTRY_SIZE;
      const info: SectionInfo = {
        id: this.view.getUint16(entryOffset + SectionEntry.ID, true) as SectionIdType,
        flags: this.view.getUint16(entryOffset + SectionEntry.FLAGS, true),
        offset: this.view.getUint32(entryOffset + SectionEntry.OFFSET, true),
        length: this.view.getUint32(entryOffset + SectionEntry.LENGTH, true),
        uncompressedLength: this.view.getUint32(entryOffset + SectionEntry.UNCOMPRESSED_LENGTH, true),
        crc32: this.view.getUint32(entryOffset + SectionEntry.CRC32, true),
      };
      this.sections.set(info.id, info);
    }
  }
  
  /**
   * Load a snapshot from an ArrayBuffer.
   * Validates the snapshot and initializes PSL.
   */
  static load(buffer: ArrayBuffer): Snapshot {
    const bytes = new Uint8Array(buffer);
    
    // Validate magic
    if (!validateMagic(bytes)) {
      throw new Error('Invalid snapshot: bad magic bytes');
    }
    
    // Validate version
    const view = new DataView(buffer);
    const version = view.getUint16(Header.VERSION, true);
    if (version !== UBX_VERSION) {
      throw new Error(`Unsupported snapshot version: ${version} (expected ${UBX_VERSION})`);
    }
    
    // Validate CRC32 if present
    const flags = view.getUint16(Header.FLAGS, true);
    if (flags & HeaderFlags.HAS_CRC32) {
      const storedCrc = view.getUint32(Header.SNAPSHOT_CRC32, true);
      // CRC32 is computed over everything except the CRC32 field itself
      const beforeCrc = bytes.subarray(0, Header.SNAPSHOT_CRC32);
      const afterCrc = bytes.subarray(Header.SNAPSHOT_CRC32 + 4);
      const combined = new Uint8Array(beforeCrc.length + afterCrc.length);
      combined.set(beforeCrc);
      combined.set(afterCrc, beforeCrc.length);
      const computedCrc = crc32(combined);
      if (storedCrc !== computedCrc) {
        throw new Error(`Snapshot CRC32 mismatch: stored=${storedCrc}, computed=${computedCrc}`);
      }
    }
    
    const snapshot = new Snapshot(buffer);
    
    // Initialize PSL from snapshot
    const pslSection = snapshot.sections.get(SectionId.PSL_SETS);
    if (pslSection) {
      const pslSets = loadPSLFromSnapshot(snapshot.view, pslSection.offset, pslSection.length);
      initPSL(pslSets);
    }
    
    return snapshot;
  }
  
  /**
   * Get the string pool section.
   */
  get strPool(): Uint8Array {
    if (!this._strPool) {
      const section = this.sections.get(SectionId.STRPOOL);
      if (!section) {
        throw new Error('Snapshot missing STRPOOL section');
      }
      // First 4 bytes are length, followed by UTF-8 bytes
      const bytesLen = this.view.getUint32(section.offset, true);
      this._strPool = new Uint8Array(this.buffer, section.offset + 4, bytesLen);
    }
    return this._strPool;
  }
  
  /**
   * Decode a string from the string pool.
   */
  getString(offset: number, length: number): string {
    const bytes = this.strPool.subarray(offset, offset + length);
    return new TextDecoder().decode(bytes);
  }
  
  /**
   * Get the domain block hash set.
   */
  get domainBlockSet(): DomainHashSet {
    if (!this._domainBlockSet) {
      const section = this.sections.get(SectionId.DOMAIN_SETS);
      if (!section) {
        // Return empty set if no domain sets
        this._domainBlockSet = new DomainHashSet(this.view, 0, true);
      } else {
        // DOMAIN_SETS contains two HashMaps: blockDomains first, then allowDomains
        this._domainBlockSet = new DomainHashSet(this.view, section.offset, false);
      }
    }
    return this._domainBlockSet;
  }
  
  /**
   * Get the domain allow hash set.
   */
  get domainAllowSet(): DomainHashSet {
    if (!this._domainAllowSet) {
      const section = this.sections.get(SectionId.DOMAIN_SETS);
      if (!section) {
        this._domainAllowSet = new DomainHashSet(this.view, 0, true);
      } else {
        // Skip past blockDomains to get to allowDomains
        const blockCapacity = this.view.getUint32(section.offset, true);
        const blockSize = HASHMAP64_HEADER_SIZE + blockCapacity * HASHMAP64_ENTRY_SIZE;
        this._domainAllowSet = new DomainHashSet(this.view, section.offset + blockSize, false);
      }
    }
    return this._domainAllowSet;
  }
  
  /**
   * Get the token dictionary.
   */
  get tokenDict(): TokenDict {
    if (!this._tokenDict) {
      const section = this.sections.get(SectionId.TOKEN_DICT);
      if (!section) {
        this._tokenDict = new TokenDict(this.view, 0, true);
      } else {
        this._tokenDict = new TokenDict(this.view, section.offset, false);
      }
    }
    return this._tokenDict;
  }
  
  /**
   * Get the token postings blob.
   */
  get tokenPostings(): Uint8Array {
    if (!this._tokenPostings) {
      const section = this.sections.get(SectionId.TOKEN_POSTINGS);
      if (!section) {
        this._tokenPostings = new Uint8Array(0);
      } else {
        const bytesLen = this.view.getUint32(section.offset, true);
        this._tokenPostings = new Uint8Array(this.buffer, section.offset + 4, bytesLen);
      }
    }
    return this._tokenPostings;
  }
  
  /**
   * Get the pattern pool.
   */
  get patternPool(): PatternPool {
    if (!this._patternPool) {
      const section = this.sections.get(SectionId.PATTERN_POOL);
      if (!section) {
        this._patternPool = new PatternPool(this.view, 0, true, this);
      } else {
        this._patternPool = new PatternPool(this.view, section.offset, false, this);
      }
    }
    return this._patternPool;
  }
  
  /**
   * Get the rules view.
   */
  get rules(): RulesView {
    if (!this._rules) {
      const section = this.sections.get(SectionId.RULES);
      if (!section) {
        this._rules = new RulesView(this.buffer, 0, 0);
      } else {
        const ruleCount = this.view.getUint32(section.offset, true);
        this._rules = new RulesView(this.buffer, section.offset + 4, ruleCount);
      }
    }
    return this._rules;
  }
  
  /**
   * Get the domain constraints blob.
   */
  get domainConstraints(): Uint8Array {
    if (!this._domainConstraints) {
      const section = this.sections.get(SectionId.DOMAIN_CONSTRAINT_POOL);
      if (!section) {
        this._domainConstraints = new Uint8Array(0);
      } else {
        const blobLen = this.view.getUint32(section.offset, true);
        this._domainConstraints = new Uint8Array(this.buffer, section.offset + 4, blobLen);
      }
    }
    return this._domainConstraints;
  }
}

// =============================================================================
// Domain Hash Set (HashMap64toU32 view)
// =============================================================================

export class DomainHashSet {
  private readonly view: DataView;
  private readonly baseOffset: number;
  private readonly capacity: number;
  private readonly isEmpty: boolean;
  
  constructor(view: DataView, offset: number, empty: boolean) {
    this.view = view;
    this.baseOffset = offset;
    this.isEmpty = empty;
    
    if (empty) {
      this.capacity = 0;
    } else {
      this.capacity = view.getUint32(offset, true);
    }
  }
  
  /**
   * Look up a domain hash and return the rule ID, or -1 if not found.
   */
  lookup(hash: Hash64): number {
    if (this.isEmpty || this.capacity === 0) {
      return -1;
    }
    
    const entriesOffset = this.baseOffset + HASHMAP64_HEADER_SIZE;
    
    // Open addressing probe
    let idx = (hash.lo >>> 0) % this.capacity;
    const mask = this.capacity - 1; // Capacity is power of 2
    
    for (let probe = 0; probe < this.capacity; probe++) {
      const entryOffset = entriesOffset + idx * HASHMAP64_ENTRY_SIZE;
      const lo = this.view.getUint32(entryOffset, true);
      const hi = this.view.getUint32(entryOffset + 4, true);
      
      // Empty slot
      if (lo === 0 && hi === 0) {
        return -1;
      }
      
      // Match found
      if (lo === hash.lo && hi === hash.hi) {
        return this.view.getUint32(entryOffset + 8, true);
      }
      
      // Linear probing
      idx = (idx + 1) & mask;
    }
    
    return -1;
  }
  
  /**
   * Check if a domain hash exists in the set.
   */
  has(hash: Hash64): boolean {
    return this.lookup(hash) !== -1;
  }
}

// =============================================================================
// Token Dictionary View
// =============================================================================

export interface TokenEntry {
  tokenHash: number;
  postingsOffset: number;
  ruleCount: number;
}

export class TokenDict {
  private readonly view: DataView;
  private readonly baseOffset: number;
  private readonly capacity: number;
  private readonly isEmpty: boolean;
  
  constructor(view: DataView, offset: number, empty: boolean) {
    this.view = view;
    this.baseOffset = offset;
    this.isEmpty = empty;
    
    if (empty) {
      this.capacity = 0;
    } else {
      this.capacity = view.getUint32(offset, true);
    }
  }
  
  /**
   * Look up a token hash and return the entry, or null if not found.
   */
  lookup(tokenHash: number): TokenEntry | null {
    if (this.isEmpty || this.capacity === 0) {
      return null;
    }
    
    const entriesOffset = this.baseOffset + TOKEN_DICT_HEADER_SIZE;
    
    // Open addressing probe
    let idx = (tokenHash >>> 0) % this.capacity;
    const mask = this.capacity - 1;
    
    for (let probe = 0; probe < this.capacity; probe++) {
      const entryOffset = entriesOffset + idx * TOKEN_DICT_ENTRY_SIZE;
      const storedHash = this.view.getUint32(entryOffset + TokenDictEntry.TOKEN_HASH, true);
      
      // Empty slot
      if (storedHash === 0) {
        return null;
      }
      
      // Match found
      if (storedHash === tokenHash) {
        return {
          tokenHash: storedHash,
          postingsOffset: this.view.getUint32(entryOffset + TokenDictEntry.POSTINGS_OFF, true),
          ruleCount: this.view.getUint32(entryOffset + TokenDictEntry.RULE_COUNT, true),
        };
      }
      
      // Linear probing
      idx = (idx + 1) & mask;
    }
    
    return null;
  }
}

// =============================================================================
// Pattern Pool View
// =============================================================================

export interface PatternEntry {
  progOffset: number;
  progLen: number;
  anchorType: number;
  flags: number;
  hostHashLo: number;
  hostHashHi: number;
}

export class PatternPool {
  private readonly view: DataView;
  private readonly baseOffset: number;
  private readonly patternCount: number;
  private readonly progBytesOffset: number;
  private readonly isEmpty: boolean;
  
  constructor(view: DataView, offset: number, empty: boolean, _snapshot: Snapshot) {
    this.view = view;
    this.baseOffset = offset;
    this.isEmpty = empty;
    
    if (empty) {
      this.patternCount = 0;
      this.progBytesOffset = 0;
    } else {
      this.patternCount = view.getUint32(offset, true);
      // Pattern index follows immediately, then progBytesLen + progBytes
      const indexSize = this.patternCount * PATTERN_INDEX_ENTRY_SIZE;
      this.progBytesOffset = offset + 4 + indexSize + 4; // +4 for progBytesLen
    }
  }
  
  /**
   * Get a pattern entry by ID.
   */
  getPattern(patternId: number): PatternEntry | null {
    if (this.isEmpty || patternId >= this.patternCount) {
      return null;
    }
    
    const entryOffset = this.baseOffset + 4 + patternId * PATTERN_INDEX_ENTRY_SIZE;
    
    return {
      progOffset: this.view.getUint32(entryOffset + PatternIndexEntry.PROG_OFF, true),
      progLen: this.view.getUint16(entryOffset + PatternIndexEntry.PROG_LEN, true),
      anchorType: this.view.getUint8(entryOffset + PatternIndexEntry.ANCHOR_TYPE),
      flags: this.view.getUint8(entryOffset + PatternIndexEntry.FLAGS),
      hostHashLo: this.view.getUint32(entryOffset + PatternIndexEntry.HOST_HASH_LO, true),
      hostHashHi: this.view.getUint32(entryOffset + PatternIndexEntry.HOST_HASH_HI, true),
    };
  }
  
  /**
   * Get the program bytes for a pattern.
   */
  getProgram(entry: PatternEntry): Uint8Array {
    if (this.isEmpty) {
      return new Uint8Array(0);
    }
    return new Uint8Array(
      this.view.buffer,
      this.progBytesOffset + entry.progOffset,
      entry.progLen
    );
  }
}

// =============================================================================
// Rules View (SoA layout)
// =============================================================================

export class RulesView {
  readonly count: number;
  
  // SoA arrays
  readonly action: Uint8Array;
  readonly flags: Uint16Array;
  readonly typeMask: Uint32Array;
  readonly partyMask: Uint8Array;
  readonly schemeMask: Uint8Array;
  readonly patternId: Uint32Array;
  readonly domainConstraintOff: Uint32Array;
  readonly optionId: Uint32Array;
  readonly priority: Int16Array;
  readonly listId: Uint16Array;
  
  constructor(buffer: ArrayBuffer, offset: number, count: number) {
    this.count = count;
    
    if (count === 0) {
      // Empty arrays
      this.action = new Uint8Array(0);
      this.flags = new Uint16Array(0);
      this.typeMask = new Uint32Array(0);
      this.partyMask = new Uint8Array(0);
      this.schemeMask = new Uint8Array(0);
      this.patternId = new Uint32Array(0);
      this.domainConstraintOff = new Uint32Array(0);
      this.optionId = new Uint32Array(0);
      this.priority = new Int16Array(0);
      this.listId = new Uint16Array(0);
      return;
    }
    
    let pos = offset;
    
    // action: u8[count]
    this.action = new Uint8Array(buffer, pos, count);
    pos = alignOffset(pos + count, 2);
    
    // flags: u16[count]
    this.flags = new Uint16Array(buffer, pos, count);
    pos = alignOffset(pos + count * 2, 4);
    
    // typeMask: u32[count]
    this.typeMask = new Uint32Array(buffer, pos, count);
    pos += count * 4;
    
    // partyMask: u8[count]
    this.partyMask = new Uint8Array(buffer, pos, count);
    pos = alignOffset(pos + count, 1);
    
    // schemeMask: u8[count]
    this.schemeMask = new Uint8Array(buffer, pos, count);
    pos = alignOffset(pos + count, 4);
    
    // patternId: u32[count]
    this.patternId = new Uint32Array(buffer, pos, count);
    pos += count * 4;
    
    // domainConstraintOff: u32[count]
    this.domainConstraintOff = new Uint32Array(buffer, pos, count);
    pos += count * 4;
    
    // optionId: u32[count]
    this.optionId = new Uint32Array(buffer, pos, count);
    pos += count * 4;
    
    // priority: i16[count]
    this.priority = new Int16Array(buffer, pos, count);
    pos = alignOffset(pos + count * 2, 2);
    
    // listId: u16[count]
    this.listId = new Uint16Array(buffer, pos, count);
  }
  
  /**
   * Check if a rule has a pattern.
   */
  hasPattern(ruleId: number): boolean {
    const patId = this.patternId[ruleId];
    return patId !== undefined && patId !== NO_PATTERN;
  }
  
  /**
   * Check if a rule has domain constraints.
   */
  hasConstraints(ruleId: number): boolean {
    const off = this.domainConstraintOff[ruleId];
    return off !== undefined && off !== NO_CONSTRAINT;
  }
}

// =============================================================================
// Varint LEB128 Decoder
// =============================================================================

/**
 * Decode a single unsigned LEB128 varint.
 * Returns [value, bytesRead].
 */
export function decodeVarint(data: Uint8Array, offset: number): [number, number] {
  let result = 0;
  let shift = 0;
  let bytesRead = 0;
  
  while (offset + bytesRead < data.length) {
    const byte = data[offset + bytesRead]!;
    bytesRead++;
    
    result |= (byte & 0x7f) << shift;
    
    if ((byte & 0x80) === 0) {
      break;
    }
    
    shift += 7;
    if (shift > 35) {
      throw new Error('Varint too long');
    }
  }
  
  return [result >>> 0, bytesRead];
}

/**
 * Decode a delta-encoded posting list.
 * Returns an array of rule IDs.
 */
export function decodePostingList(
  data: Uint8Array,
  offset: number,
  count: number
): Uint32Array {
  const result = new Uint32Array(count);
  let pos = offset;
  let prevId = 0;
  
  for (let i = 0; i < count; i++) {
    const [delta, bytesRead] = decodeVarint(data, pos);
    pos += bytesRead;
    prevId += delta;
    result[i] = prevId;
  }
  
  return result;
}
