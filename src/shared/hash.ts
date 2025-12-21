/**
 * Hash functions for BetterBlocker
 * 
 * Uses Murmur3 32-bit with two different seeds to create a 64-bit composite key.
 * This provides excellent distribution with virtually no collision risk for domains.
 * 
 * Empty slot sentinel in hash tables is (lo=0, hi=0).
 * We ensure this never occurs by OR-ing lo |= 1 after hashing.
 */

import type { Hash64 } from './types.js';

// Default seeds for the two hash functions
const SEED_LO = 0x9e3779b9; // Golden ratio
const SEED_HI = 0x85ebca6b; // Murmer3 constant

/**
 * Murmur3 32-bit hash implementation.
 * Optimized for short strings (typical domain lengths).
 */
export function murmur3_32(str: string, seed: number): number {
  const len = str.length;
  let h = seed;
  let k: number;
  let i = 0;

  // Process 4-character chunks
  const chunks = (len >> 2) << 2; // Round down to multiple of 4
  while (i < chunks) {
    k =
      (str.charCodeAt(i) & 0xff) |
      ((str.charCodeAt(i + 1) & 0xff) << 8) |
      ((str.charCodeAt(i + 2) & 0xff) << 16) |
      ((str.charCodeAt(i + 3) & 0xff) << 24);

    k = Math.imul(k, 0xcc9e2d51);
    k = (k << 15) | (k >>> 17);
    k = Math.imul(k, 0x1b873593);

    h ^= k;
    h = (h << 13) | (h >>> 19);
    h = Math.imul(h, 5) + 0xe6546b64;

    i += 4;
  }

  // Process remaining bytes (using cascading if instead of switch fallthrough)
  k = 0;
  const remainder = len & 3;
  if (remainder >= 3) {
    k ^= (str.charCodeAt(i + 2) & 0xff) << 16;
  }
  if (remainder >= 2) {
    k ^= (str.charCodeAt(i + 1) & 0xff) << 8;
  }
  if (remainder >= 1) {
    k ^= str.charCodeAt(i) & 0xff;
    k = Math.imul(k, 0xcc9e2d51);
    k = (k << 15) | (k >>> 17);
    k = Math.imul(k, 0x1b873593);
    h ^= k;
  }

  // Finalization
  h ^= len;
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;

  return h >>> 0; // Ensure unsigned 32-bit
}

/**
 * Compute 64-bit hash as (lo, hi) pair using two Murmur3 passes.
 * Ensures the result is never (0, 0) by OR-ing lo with 1.
 */
export function hash64(str: string): Hash64 {
  let lo = murmur3_32(str, SEED_LO);
  const hi = murmur3_32(str, SEED_HI);

  // Avoid (0, 0) sentinel by ensuring lo is never 0 when hi is 0
  if (lo === 0 && hi === 0) {
    lo = 1;
  }

  return { lo, hi };
}

/**
 * Compute a 32-bit hash for tokens.
 * Uses a single Murmur3 pass with a different seed.
 * Ensures result is never 0 (sentinel value).
 */
export function hashToken(str: string): number {
  let h = murmur3_32(str, 0x811c9dc5); // FNV offset basis as seed
  if (h === 0) {
    h = 1;
  }
  return h;
}

/**
 * Hash a domain string for lookup in domain sets.
 * Lowercases the input before hashing for case-insensitive matching.
 */
export function hashDomain(domain: string): Hash64 {
  return hash64(domain.toLowerCase());
}

/**
 * Compare two Hash64 values for equality.
 */
export function hash64Equals(a: Hash64, b: Hash64): boolean {
  return a.lo === b.lo && a.hi === b.hi;
}

/**
 * Check if a Hash64 is the empty sentinel (0, 0).
 */
export function hash64IsEmpty(h: Hash64): boolean {
  return h.lo === 0 && h.hi === 0;
}

/**
 * Pack Hash64 into a BigInt for use as Map key.
 */
export function hash64ToBigInt(h: Hash64): bigint {
  return (BigInt(h.hi) << 32n) | BigInt(h.lo >>> 0);
}

/**
 * Unpack BigInt back to Hash64.
 */
export function bigIntToHash64(n: bigint): Hash64 {
  return {
    lo: Number(n & 0xffffffffn),
    hi: Number((n >> 32n) & 0xffffffffn),
  };
}

/**
 * Hash bytes from a Uint8Array.
 * Used for hashing binary data in snapshot verification.
 */
export function murmur3_32_bytes(data: Uint8Array, seed: number): number {
  const len = data.length;
  let h = seed;
  let k: number;
  let i = 0;

  // Process 4-byte chunks
  const chunks = (len >> 2) << 2;
  while (i < chunks) {
    k =
      data[i]! |
      (data[i + 1]! << 8) |
      (data[i + 2]! << 16) |
      (data[i + 3]! << 24);

    k = Math.imul(k, 0xcc9e2d51);
    k = (k << 15) | (k >>> 17);
    k = Math.imul(k, 0x1b873593);

    h ^= k;
    h = (h << 13) | (h >>> 19);
    h = Math.imul(h, 5) + 0xe6546b64;

    i += 4;
  }

  // Process remaining bytes (using cascading if instead of switch fallthrough)
  k = 0;
  const remainder = len & 3;
  if (remainder >= 3) {
    k ^= data[i + 2]! << 16;
  }
  if (remainder >= 2) {
    k ^= data[i + 1]! << 8;
  }
  if (remainder >= 1) {
    k ^= data[i]!;
    k = Math.imul(k, 0xcc9e2d51);
    k = (k << 15) | (k >>> 17);
    k = Math.imul(k, 0x1b873593);
    h ^= k;
  }

  // Finalization
  h ^= len;
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;

  return h >>> 0;
}

/**
 * Compute CRC32 for snapshot integrity checking.
 * Uses the standard CRC32 polynomial (IEEE 802.3).
 */
const CRC32_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let j = 0; j < 8; j++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[i] = c;
  }
  return table;
})();

export function crc32(data: Uint8Array): number {
  let crc = 0xffffffff;
  for (let i = 0; i < data.length; i++) {
    crc = CRC32_TABLE[(crc ^ data[i]!) & 0xff]! ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}
