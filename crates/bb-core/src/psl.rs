//! Public Suffix List (PSL) utilities for eTLD+1 extraction
//!
//! This module provides fast eTLD+1 extraction with LRU caching.
//! The PSL data is loaded from the snapshot at runtime.
//!
//! # Examples
//!
//! ```
//! use bb_core::psl::get_etld1;
//!
//! assert_eq!(get_etld1("sub.example.com"), "example.com");
//! assert_eq!(get_etld1("sub.example.co.uk"), "example.co.uk");
//! ```

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec, collections::BTreeSet};

#[cfg(feature = "std")]
use std::collections::HashSet;

use crate::hash::{Hash64, hash_domain};

// =============================================================================
// LRU Cache
// =============================================================================

/// Simple fixed-size cache for eTLD+1 lookups.
/// Uses a basic LRU strategy with a hashmap + vec.
#[cfg(feature = "std")]
pub struct LruCache {
    capacity: usize,
    entries: std::collections::HashMap<String, String>,
    order: std::collections::VecDeque<String>,
}

#[cfg(feature = "std")]
impl LruCache {
    /// Create a new LRU cache with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: std::collections::HashMap::with_capacity(capacity),
            order: std::collections::VecDeque::with_capacity(capacity),
        }
    }

    /// Get a value from the cache.
    pub fn get(&mut self, key: &str) -> Option<&str> {
        if self.entries.contains_key(key) {
            // Move to back (most recently used)
            self.order.retain(|k| k != key);
            self.order.push_back(key.to_string());
            self.entries.get(key).map(|s| s.as_str())
        } else {
            None
        }
    }

    /// Insert a value into the cache.
    pub fn insert(&mut self, key: String, value: String) {
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&key) {
            // Evict oldest
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    /// Clear the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

// =============================================================================
// PSL Hash Sets
// =============================================================================

/// PSL rule sets for suffix lookup.
#[derive(Debug, Default)]
pub struct PslSets {
    /// Exact TLD rules (e.g., "com", "co.uk")
    #[cfg(feature = "std")]
    pub exact: HashSet<u64>,
    #[cfg(not(feature = "std"))]
    pub exact: BTreeSet<u64>,
    
    /// Wildcard rules (e.g., "*.ck" stored as "ck")
    #[cfg(feature = "std")]
    pub wildcard: HashSet<u64>,
    #[cfg(not(feature = "std"))]
    pub wildcard: BTreeSet<u64>,
    
    /// Exception rules (e.g., "!www.ck" stored as "www.ck")
    #[cfg(feature = "std")]
    pub exception: HashSet<u64>,
    #[cfg(not(feature = "std"))]
    pub exception: BTreeSet<u64>,
}

impl PslSets {
    /// Create empty PSL sets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a suffix is an exact TLD rule.
    #[inline]
    pub fn is_exact(&self, suffix: &str) -> bool {
        let hash = hash_domain(suffix);
        self.exact.contains(&hash.to_u64())
    }

    /// Check if a suffix matches a wildcard rule.
    #[inline]
    pub fn is_wildcard(&self, suffix: &str) -> bool {
        let hash = hash_domain(suffix);
        self.wildcard.contains(&hash.to_u64())
    }

    /// Check if a suffix is an exception rule.
    #[inline]
    pub fn is_exception(&self, suffix: &str) -> bool {
        let hash = hash_domain(suffix);
        self.exception.contains(&hash.to_u64())
    }
}

// =============================================================================
// Global PSL State
// =============================================================================

#[cfg(feature = "std")]
use std::sync::RwLock;

#[cfg(feature = "std")]
static PSL_SETS: RwLock<Option<PslSets>> = RwLock::new(None);

#[cfg(feature = "std")]
static ETLD1_CACHE: RwLock<Option<LruCache>> = RwLock::new(None);

/// Initialize PSL from sets.
#[cfg(feature = "std")]
pub fn init_psl(sets: PslSets) {
    *PSL_SETS.write().unwrap() = Some(sets);
    *ETLD1_CACHE.write().unwrap() = Some(LruCache::new(4096));
}

/// Check if PSL is initialized.
#[cfg(feature = "std")]
pub fn is_psl_initialized() -> bool {
    PSL_SETS.read().unwrap().is_some()
}

// =============================================================================
// eTLD+1 Extraction
// =============================================================================

/// Common two-part TLDs for fallback.
const COMMON_TWO_PART_TLDS: &[&str] = &[
    "co.uk", "co.jp", "co.nz", "co.za", "co.in", "co.kr",
    "com.au", "com.br", "com.cn", "com.mx", "com.tw", "com.hk",
    "net.au", "net.nz",
    "org.uk", "org.au",
    "gov.uk", "gov.au",
    "ac.uk", "ac.jp",
    "ne.jp", "or.jp",
];

/// Get the eTLD+1 (registrable domain) for a hostname.
///
/// If PSL is not loaded, falls back to simple heuristic.
#[cfg(feature = "std")]
pub fn get_etld1(host: &str) -> String {
    let host = host.to_lowercase();
    let host = host.trim_end_matches('.');

    // Check cache
    if let Some(ref mut cache) = *ETLD1_CACHE.write().unwrap() {
        if let Some(cached) = cache.get(host) {
            return cached.to_string();
        }
    }

    let result = compute_etld1(host);

    // Store in cache
    if let Some(ref mut cache) = *ETLD1_CACHE.write().unwrap() {
        cache.insert(host.to_string(), result.clone());
    }

    result
}

/// Compute eTLD+1 without caching.
#[cfg(feature = "std")]
fn compute_etld1(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();

    if n <= 1 {
        return host.to_string();
    }

    // Check PSL if available
    if let Some(ref psl) = *PSL_SETS.read().unwrap() {
        for i in 0..n - 1 {
            let suffix: String = labels[i..].join(".");
            let parent_suffix: String = if i + 1 < n {
                labels[i + 1..].join(".")
            } else {
                String::new()
            };

            // Exception rules override wildcards
            if psl.is_exception(&suffix) {
                if i > 0 {
                    return labels[i - 1..].join(".");
                }
                return suffix;
            }

            // Exact rule
            if psl.is_exact(&suffix) {
                if i > 0 {
                    return labels[i - 1..].join(".");
                }
                return host.to_string();
            }

            // Wildcard rule on parent
            if !parent_suffix.is_empty() && psl.is_wildcard(&parent_suffix) {
                if i > 0 {
                    return labels[i - 1..].join(".");
                }
                return suffix;
            }
        }
    }

    // Fallback heuristic
    fallback_etld1(&labels)
}

/// Fallback eTLD+1 heuristic.
fn fallback_etld1(labels: &[&str]) -> String {
    let n = labels.len();
    if n <= 2 {
        return labels.join(".");
    }

    // Check for common two-part TLDs
    let last_two = format!("{}.{}", labels[n - 2], labels[n - 1]);
    if COMMON_TWO_PART_TLDS.contains(&last_two.as_str()) {
        return labels[n - 3..].join(".");
    }

    // Default: last 2 labels
    labels[n - 2..].join(".")
}

/// Check if two hosts share the same eTLD+1.
#[cfg(feature = "std")]
pub fn is_same_site(host1: &str, host2: &str) -> bool {
    get_etld1(host1) == get_etld1(host2)
}

/// Check if a request is third-party.
#[cfg(feature = "std")]
pub fn is_third_party(site_host: &str, req_host: &str) -> bool {
    get_etld1(site_host) != get_etld1(req_host)
}

/// Get the parent domain (strip leftmost label).
pub fn get_parent_domain(host: &str) -> Option<&str> {
    match host.find('.') {
        Some(idx) if idx < host.len() - 1 => Some(&host[idx + 1..]),
        _ => None,
    }
}

/// Iterator for suffix-walking a host from full to eTLD+1.
pub struct HostSuffixIter<'a> {
    current: &'a str,
    etld1_len: usize,
}

impl<'a> HostSuffixIter<'a> {
    #[cfg(feature = "std")]
    pub fn new(host: &'a str) -> Self {
        let etld1 = get_etld1(host);
        Self {
            current: host,
            etld1_len: etld1.len(),
        }
    }
}

impl<'a> Iterator for HostSuffixIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.len() < self.etld1_len {
            return None;
        }

        let result = self.current;

        // Move to parent
        if let Some(parent) = get_parent_domain(self.current) {
            if parent.len() >= self.etld1_len {
                self.current = parent;
            } else {
                self.current = "";
            }
        } else {
            self.current = "";
        }

        Some(result)
    }
}

/// Walk host suffixes from most specific to least specific.
#[cfg(feature = "std")]
pub fn walk_host_suffixes(host: &str) -> HostSuffixIter<'_> {
    HostSuffixIter::new(host)
}

// =============================================================================
// PSL Loading from Snapshot
// =============================================================================

/// Load PSL sets from snapshot data.
pub fn load_psl_from_bytes(data: &[u8], offset: usize) -> PslSets {
    let mut sets = PslSets::new();
    let mut pos = offset;

    // Load each of the three hash sets
    for set in [&mut sets.exact, &mut sets.wildcard, &mut sets.exception] {
        if pos + 20 > data.len() {
            break;
        }

        let capacity = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        // count, seedLo, seedHi, reserved at pos+4..pos+20
        pos += 20;

        for _ in 0..capacity {
            if pos + 8 > data.len() {
                break;
            }

            let lo = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            let hi = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
            pos += 8;

            // Skip empty slots
            if lo != 0 || hi != 0 {
                let hash64 = Hash64::new(lo, hi);
                set.insert(hash64.to_u64());
            }
        }
    }

    sets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_etld1_simple() {
        assert_eq!(fallback_etld1(&["example", "com"]), "example.com");
        assert_eq!(fallback_etld1(&["sub", "example", "com"]), "example.com");
    }

    #[test]
    fn test_fallback_etld1_two_part() {
        assert_eq!(fallback_etld1(&["sub", "example", "co", "uk"]), "example.co.uk");
        assert_eq!(fallback_etld1(&["example", "co", "uk"]), "example.co.uk");
    }

    #[test]
    fn test_get_parent_domain() {
        assert_eq!(get_parent_domain("sub.example.com"), Some("example.com"));
        assert_eq!(get_parent_domain("example.com"), Some("com"));
        assert_eq!(get_parent_domain("com"), None);
        assert_eq!(get_parent_domain(""), None);
    }
}
