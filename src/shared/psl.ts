/**
 * Public Suffix List (PSL) utilities for eTLD+1 extraction
 * 
 * This module provides fast eTLD+1 extraction with LRU caching.
 * The PSL data is loaded from the snapshot at runtime.
 * 
 * eTLD+1 = effective top-level domain + 1 label
 * Examples:
 *   - example.com → example.com (com is eTLD)
 *   - sub.example.com → example.com
 *   - example.co.uk → example.co.uk (co.uk is eTLD)
 *   - sub.example.co.uk → example.co.uk
 */

import type { Hash64 } from './types.js';
import { hashDomain } from './hash.js';

// =============================================================================
// LRU Cache
// =============================================================================

/**
 * Fixed-size LRU cache with O(1) operations.
 * Uses a doubly-linked list + Map for fast access and eviction.
 */
export class LRUCache<K, V> {
  private readonly capacity: number;
  private readonly cache: Map<K, V>;

  constructor(capacity: number) {
    this.capacity = capacity;
    this.cache = new Map();
  }

  get(key: K): V | undefined {
    const value = this.cache.get(key);
    if (value !== undefined) {
      // Move to end (most recently used) by delete + set
      this.cache.delete(key);
      this.cache.set(key, value);
    }
    return value;
  }

  set(key: K, value: V): void {
    // If key exists, delete it first (to update position)
    if (this.cache.has(key)) {
      this.cache.delete(key);
    } else if (this.cache.size >= this.capacity) {
      // Evict oldest (first) entry
      const firstKey = this.cache.keys().next().value as K;
      this.cache.delete(firstKey);
    }
    this.cache.set(key, value);
  }

  has(key: K): boolean {
    return this.cache.has(key);
  }

  clear(): void {
    this.cache.clear();
  }

  get size(): number {
    return this.cache.size;
  }
}

// =============================================================================
// PSL Hash Sets (populated from snapshot)
// =============================================================================

/**
 * PSL rule sets for suffix lookup.
 * These are populated from the PSL_SETS section of the snapshot.
 */
export interface PSLSets {
  /** Exact TLD rules (e.g., "com", "co.uk") */
  exact: Set<bigint>;
  /** Wildcard rules (e.g., "*.ck" meaning all .ck subdomains are TLDs) */
  wildcard: Set<bigint>;
  /** Exception rules (e.g., "!www.ck" meaning www.ck is NOT a TLD) */
  exception: Set<bigint>;
}

// Global PSL data (loaded from snapshot)
let pslSets: PSLSets | null = null;

// eTLD+1 cache
const etld1Cache = new LRUCache<string, string>(4096);

/**
 * Initialize PSL from sets.
 * Called when loading snapshot.
 */
export function initPSL(sets: PSLSets): void {
  pslSets = sets;
  etld1Cache.clear();
}

/**
 * Check if PSL is initialized.
 */
export function isPSLInitialized(): boolean {
  return pslSets !== null;
}

// =============================================================================
// eTLD+1 Extraction
// =============================================================================

/**
 * Split hostname into labels (e.g., "sub.example.com" → ["sub", "example", "com"])
 */
function splitHost(host: string): string[] {
  // Remove trailing dot if present
  if (host.endsWith('.')) {
    host = host.slice(0, -1);
  }
  return host.toLowerCase().split('.');
}

/**
 * Convert Hash64 to bigint for Set lookup.
 */
function hash64ToBigInt(h: Hash64): bigint {
  return (BigInt(h.hi) << 32n) | BigInt(h.lo >>> 0);
}

/**
 * Check if a suffix is in the PSL exact set.
 */
function isExactRule(suffix: string): boolean {
  if (!pslSets) return false;
  const hash = hashDomain(suffix);
  return pslSets.exact.has(hash64ToBigInt(hash));
}

/**
 * Check if a suffix matches a wildcard rule.
 * For "*.foo.bar", we check if "foo.bar" is in the wildcard set.
 */
function isWildcardRule(suffix: string): boolean {
  if (!pslSets) return false;
  const hash = hashDomain(suffix);
  return pslSets.wildcard.has(hash64ToBigInt(hash));
}

/**
 * Check if a suffix is an exception to a wildcard rule.
 */
function isExceptionRule(suffix: string): boolean {
  if (!pslSets) return false;
  const hash = hashDomain(suffix);
  return pslSets.exception.has(hash64ToBigInt(hash));
}

/**
 * Get the eTLD+1 (registrable domain) for a hostname.
 * 
 * Algorithm (per publicsuffix.org):
 * 1. Match domain against PSL rules
 * 2. Return the TLD + one label
 * 
 * If PSL is not loaded, falls back to simple heuristic (last 2 labels).
 */
export function getETLD1(host: string): string {
  // Normalize
  host = host.toLowerCase();
  if (host.endsWith('.')) {
    host = host.slice(0, -1);
  }

  // Check cache first
  const cached = etld1Cache.get(host);
  if (cached !== undefined) {
    return cached;
  }

  const result = computeETLD1(host);
  etld1Cache.set(host, result);
  return result;
}

/**
 * Compute eTLD+1 without caching (for internal use).
 */
function computeETLD1(host: string): string {
  const labels = splitHost(host);
  const n = labels.length;

  // Single label (e.g., "localhost") is its own eTLD+1
  if (n <= 1) {
    return host;
  }

  // If PSL not loaded, use simple heuristic
  if (!pslSets) {
    return fallbackETLD1(labels);
  }

  // Find the longest matching suffix that is a TLD
  // Start from the rightmost label and work left
  for (let i = 0; i < n - 1; i++) {
    const suffix = labels.slice(i).join('.');
    const parentSuffix = labels.slice(i + 1).join('.');

    // Check exception rule first (exceptions override wildcards)
    if (isExceptionRule(suffix)) {
      // This suffix is an exception, so parentSuffix is the TLD
      // eTLD+1 is one label to the left of parentSuffix
      if (i > 0) {
        return labels.slice(i - 1).join('.');
      }
      return suffix;
    }

    // Check exact rule
    if (isExactRule(suffix)) {
      // This suffix is a TLD, eTLD+1 is one label to the left
      if (i > 0) {
        return labels.slice(i - 1).join('.');
      }
      // The entire host is a TLD (unusual but possible)
      return host;
    }

    // Check wildcard rule on parent
    if (parentSuffix && isWildcardRule(parentSuffix)) {
      // *.parentSuffix means suffix is a TLD
      // eTLD+1 is one label to the left
      if (i > 0) {
        return labels.slice(i - 1).join('.');
      }
      return suffix;
    }
  }

  // No PSL match, use fallback
  return fallbackETLD1(labels);
}

/**
 * Fallback eTLD+1 heuristic when PSL is not available.
 * Uses common multi-part TLDs.
 */
function fallbackETLD1(labels: string[]): string {
  const n = labels.length;
  if (n <= 2) {
    return labels.join('.');
  }

  // Check for common two-part TLDs
  const lastTwo = labels.slice(-2).join('.');
  const commonTwoPartTLDs = new Set([
    'co.uk', 'co.jp', 'co.nz', 'co.za', 'co.in', 'co.kr',
    'com.au', 'com.br', 'com.cn', 'com.mx', 'com.tw', 'com.hk',
    'net.au', 'net.nz',
    'org.uk', 'org.au',
    'gov.uk', 'gov.au',
    'ac.uk', 'ac.jp',
    'ne.jp', 'or.jp',
  ]);

  if (commonTwoPartTLDs.has(lastTwo)) {
    // Return last 3 labels
    return labels.slice(-3).join('.');
  }

  // Default: last 2 labels
  return labels.slice(-2).join('.');
}

/**
 * Check if two hosts share the same eTLD+1 (same-site check).
 */
export function isSameSite(host1: string, host2: string): boolean {
  return getETLD1(host1) === getETLD1(host2);
}

/**
 * Check if a request is third-party based on site and request hosts.
 */
export function isThirdParty(siteHost: string, reqHost: string): boolean {
  return getETLD1(siteHost) !== getETLD1(reqHost);
}

/**
 * Get the parent domain (strip leftmost label).
 */
export function getParentDomain(host: string): string | null {
  const idx = host.indexOf('.');
  if (idx === -1 || idx === host.length - 1) {
    return null;
  }
  return host.slice(idx + 1);
}

/**
 * Iterator for suffix-walking a host from full to eTLD+1.
 * Yields: full host, then parent, then grandparent, etc.
 * Stops at eTLD+1 (does not yield TLD alone).
 */
export function* walkHostSuffixes(host: string): Generator<string> {
  const etld1 = getETLD1(host);
  let current = host.toLowerCase();
  
  while (current.length >= etld1.length) {
    yield current;
    const parent = getParentDomain(current);
    if (!parent || parent.length < etld1.length) {
      break;
    }
    current = parent;
  }
}

/**
 * Get all suffixes of a host from most specific to least specific (but not below eTLD+1).
 */
export function getHostSuffixes(host: string): string[] {
  return Array.from(walkHostSuffixes(host));
}

// =============================================================================
// PSL Loading from Snapshot (hash set views)
// =============================================================================

/**
 * Load PSL sets from a snapshot's PSL_SETS section.
 * The section contains three HashSet64 structures.
 */
export function loadPSLFromSnapshot(
  data: DataView,
  offset: number,
  _length: number
): PSLSets {
  const sets: PSLSets = {
    exact: new Set(),
    wildcard: new Set(),
    exception: new Set(),
  };

  let pos = offset;

  // Load each of the three hash sets
  for (const setName of ['exact', 'wildcard', 'exception'] as const) {
    const capacity = data.getUint32(pos, true);
    // count is at pos+4, seedLo at pos+8, seedHi at pos+12, reserved at pos+16
    pos += 20; // Skip header (capacity + count + seedLo + seedHi + reserved)

    const set = sets[setName];
    for (let i = 0; i < capacity; i++) {
      const lo = data.getUint32(pos, true);
      const hi = data.getUint32(pos + 4, true);
      pos += 8;

      // Skip empty slots (sentinel is 0,0)
      if (lo !== 0 || hi !== 0) {
        const bigint = (BigInt(hi) << 32n) | BigInt(lo >>> 0);
        set.add(bigint);
      }
    }
  }

  return sets;
}
