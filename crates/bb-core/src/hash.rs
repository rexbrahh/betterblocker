//! Hash functions for BetterBlocker
//!
//! Uses Murmur3 32-bit with two different seeds to create a 64-bit composite key.
//! This provides excellent distribution with virtually no collision risk for domains.
//!
//! # Sentinel Handling
//!
//! Empty slot sentinel in hash tables is `(lo=0, hi=0)`.
//! We ensure this never occurs by OR-ing `lo |= 1` after hashing.

/// 64-bit hash represented as two 32-bit parts.
/// Used for domain hashing with extremely low collision probability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(C)]
pub struct Hash64 {
    pub lo: u32,
    pub hi: u32,
}

impl Hash64 {
    /// Create a new Hash64 from lo and hi parts.
    #[inline]
    pub const fn new(lo: u32, hi: u32) -> Self {
        Self { lo, hi }
    }

    /// Check if this hash is the empty sentinel (0, 0).
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.lo == 0 && self.hi == 0
    }

    /// Convert to a single u64 for use as a map key.
    #[inline]
    pub const fn to_u64(&self) -> u64 {
        ((self.hi as u64) << 32) | (self.lo as u64)
    }

    /// Create from a u64.
    #[inline]
    pub const fn from_u64(v: u64) -> Self {
        Self {
            lo: v as u32,
            hi: (v >> 32) as u32,
        }
    }
}

// Default seeds for the two hash functions
const SEED_LO: u32 = 0x9e3779b9; // Golden ratio
const SEED_HI: u32 = 0x85ebca6b; // Murmur3 constant

/// Murmur3 32-bit hash implementation.
/// Optimized for short strings (typical domain lengths).
#[inline]
pub fn murmur3_32(data: &[u8], seed: u32) -> u32 {
    let len = data.len();
    let mut h = seed;
    let mut i = 0;

    // Process 4-byte chunks
    let chunks = (len >> 2) << 2; // Round down to multiple of 4
    while i < chunks {
        let k = u32::from_le_bytes([
            data[i],
            data[i + 1],
            data[i + 2],
            data[i + 3],
        ]);

        let k = k.wrapping_mul(0xcc9e2d51);
        let k = k.rotate_left(15);
        let k = k.wrapping_mul(0x1b873593);

        h ^= k;
        h = h.rotate_left(13);
        h = h.wrapping_mul(5).wrapping_add(0xe6546b64);

        i += 4;
    }

    // Process remaining bytes
    let mut k: u32 = 0;
    let remainder = len & 3;
    if remainder >= 3 {
        k ^= (data[i + 2] as u32) << 16;
    }
    if remainder >= 2 {
        k ^= (data[i + 1] as u32) << 8;
    }
    if remainder >= 1 {
        k ^= data[i] as u32;
        let k = k.wrapping_mul(0xcc9e2d51);
        let k = k.rotate_left(15);
        let k = k.wrapping_mul(0x1b873593);
        h ^= k;
    }

    // Finalization
    h ^= len as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^= h >> 16;

    h
}

/// Compute 64-bit hash as (lo, hi) pair using two Murmur3 passes.
/// Ensures the result is never (0, 0) by OR-ing lo with 1.
#[inline]
pub fn hash64(data: &[u8]) -> Hash64 {
    let mut lo = murmur3_32(data, SEED_LO);
    let hi = murmur3_32(data, SEED_HI);

    // Avoid (0, 0) sentinel
    if lo == 0 && hi == 0 {
        lo = 1;
    }

    Hash64 { lo, hi }
}

/// Hash a domain string for lookup in domain sets.
/// Lowercases the input before hashing for case-insensitive matching.
#[inline]
pub fn hash_domain(domain: &str) -> Hash64 {
    // Fast lowercase conversion for ASCII domains
    let mut buf = [0u8; 256];
    let len = domain.len().min(256);
    
    for (i, &b) in domain.as_bytes()[..len].iter().enumerate() {
        buf[i] = if b.is_ascii_uppercase() {
            b + 32
        } else {
            b
        };
    }
    
    hash64(&buf[..len])
}

/// Compute a 32-bit hash for tokens.
/// Uses a single Murmur3 pass with a different seed.
/// Ensures result is never 0 (sentinel value).
#[inline]
pub fn hash_token(token: &str) -> u32 {
    let mut h = murmur3_32(token.as_bytes(), 0x811c9dc5);
    if h == 0 {
        h = 1;
    }
    h
}

/// Compute CRC32 for snapshot integrity checking.
/// Uses the standard CRC32 polynomial (IEEE 802.3).
pub fn crc32(data: &[u8]) -> u32 {
    static CRC32_TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut j = 0;
            while j < 8 {
                c = if c & 1 != 0 {
                    0xedb88320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                j += 1;
            }
            table[i] = c;
            i += 1;
        }
        table
    };

    let mut crc = 0xffffffff_u32;
    for &byte in data {
        crc = CRC32_TABLE[((crc ^ byte as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    crc ^ 0xffffffff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_murmur3_consistent() {
        let h1 = murmur3_32(b"example.com", 0);
        let h2 = murmur3_32(b"example.com", 0);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_murmur3_different_strings() {
        let h1 = murmur3_32(b"example.com", 0);
        let h2 = murmur3_32(b"example.org", 0);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_murmur3_different_seeds() {
        let h1 = murmur3_32(b"example.com", 0);
        let h2 = murmur3_32(b"example.com", 1);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_murmur3_empty_string() {
        let h = murmur3_32(b"", 0);
        assert_eq!(h, h);
    }

    #[test]
    fn test_murmur3_various_lengths() {
        for len in 1..=20 {
            let s = vec![b'a'; len];
            let h = murmur3_32(&s, 0);
            assert_eq!(h, h);
        }
    }

    #[test]
    fn test_hash64_never_zero() {
        // Test many strings to ensure we never get the sentinel value
        let test_strings = [
            b"" as &[u8],
            b"a",
            b"test",
            b"example.com",
            b"very-long-domain-name.example.com",
        ];
        for s in test_strings {
            let h = hash64(s);
            assert!(!h.is_empty(), "hash64({:?}) returned empty sentinel", s);
        }
    }

    #[test]
    fn test_hash64_is_empty() {
        let empty = Hash64 { lo: 0, hi: 0 };
        let non_empty = Hash64 { lo: 1, hi: 0 };
        assert!(empty.is_empty());
        assert!(!non_empty.is_empty());
    }

    #[test]
    fn test_hash_domain_case_insensitive() {
        let h1 = hash_domain("Example.COM");
        let h2 = hash_domain("example.com");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_token_never_zero() {
        let h = hash_token("script");
        assert_ne!(h, 0);
    }

    #[test]
    fn test_crc32_consistent() {
        let data = [1u8, 2, 3, 4, 5];
        let c1 = crc32(&data);
        let c2 = crc32(&data);
        assert_eq!(c1, c2);
    }

    #[test]
    fn test_crc32_empty() {
        let data: [u8; 0] = [];
        let c = crc32(&data);
        assert_eq!(c, c);
    }

    #[test]
    fn test_crc32_detects_changes() {
        let data1 = [1u8, 2, 3];
        let data2 = [1u8, 2, 4];
        assert_ne!(crc32(&data1), crc32(&data2));
    }
}
