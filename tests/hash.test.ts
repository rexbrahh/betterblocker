import { describe, it, expect } from 'bun:test';
import {
  murmur3_32,
  hash64,
  hashToken,
  hashDomain,
  hash64Equals,
  hash64IsEmpty,
  crc32,
} from '../src/shared/hash.js';

describe('murmur3_32', () => {
  it('should return consistent hashes', () => {
    const h1 = murmur3_32('example.com', 0);
    const h2 = murmur3_32('example.com', 0);
    expect(h1).toBe(h2);
  });

  it('should return different hashes for different strings', () => {
    const h1 = murmur3_32('example.com', 0);
    const h2 = murmur3_32('example.org', 0);
    expect(h1).not.toBe(h2);
  });

  it('should return different hashes for different seeds', () => {
    const h1 = murmur3_32('example.com', 0);
    const h2 = murmur3_32('example.com', 1);
    expect(h1).not.toBe(h2);
  });

  it('should handle empty strings', () => {
    const h = murmur3_32('', 0);
    expect(typeof h).toBe('number');
    expect(h).toBeGreaterThanOrEqual(0);
  });

  it('should handle strings of various lengths', () => {
    for (let len = 1; len <= 20; len++) {
      const str = 'a'.repeat(len);
      const h = murmur3_32(str, 0);
      expect(typeof h).toBe('number');
      expect(h >>> 0).toBe(h); // Should be unsigned 32-bit
    }
  });
});

describe('hash64', () => {
  it('should return (lo, hi) pair', () => {
    const h = hash64('example.com');
    expect(typeof h.lo).toBe('number');
    expect(typeof h.hi).toBe('number');
  });

  it('should never return (0, 0)', () => {
    // Test many strings to ensure we never get the sentinel value
    const testStrings = [
      '',
      'a',
      'test',
      'example.com',
      'very-long-domain-name.example.com',
    ];
    for (const str of testStrings) {
      const h = hash64(str);
      expect(h.lo !== 0 || h.hi !== 0).toBe(true);
    }
  });

  it('should be consistent', () => {
    const h1 = hash64('test');
    const h2 = hash64('test');
    expect(hash64Equals(h1, h2)).toBe(true);
  });
});

describe('hashToken', () => {
  it('should return non-zero values', () => {
    const h = hashToken('script');
    expect(h).not.toBe(0);
  });

  it('should be consistent', () => {
    const h1 = hashToken('analytics');
    const h2 = hashToken('analytics');
    expect(h1).toBe(h2);
  });
});

describe('hashDomain', () => {
  it('should be case-insensitive', () => {
    const h1 = hashDomain('Example.COM');
    const h2 = hashDomain('example.com');
    expect(hash64Equals(h1, h2)).toBe(true);
  });
});

describe('hash64IsEmpty', () => {
  it('should return true for (0, 0)', () => {
    expect(hash64IsEmpty({ lo: 0, hi: 0 })).toBe(true);
  });

  it('should return false for non-zero values', () => {
    expect(hash64IsEmpty({ lo: 1, hi: 0 })).toBe(false);
    expect(hash64IsEmpty({ lo: 0, hi: 1 })).toBe(false);
    expect(hash64IsEmpty({ lo: 1, hi: 1 })).toBe(false);
  });
});

describe('crc32', () => {
  it('should return consistent values', () => {
    const data = new Uint8Array([1, 2, 3, 4, 5]);
    const c1 = crc32(data);
    const c2 = crc32(data);
    expect(c1).toBe(c2);
  });

  it('should handle empty data', () => {
    const c = crc32(new Uint8Array(0));
    expect(typeof c).toBe('number');
  });

  it('should detect changes', () => {
    const data1 = new Uint8Array([1, 2, 3]);
    const data2 = new Uint8Array([1, 2, 4]);
    expect(crc32(data1)).not.toBe(crc32(data2));
  });
});
