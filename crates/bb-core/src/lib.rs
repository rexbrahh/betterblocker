//! BetterBlocker Core Library
//!
//! This crate provides the core matching engine for the BetterBlocker content blocker.
//! It is designed to be `no_std` compatible (with `alloc`) for maximum portability.
//!
//! # Architecture
//!
//! The matching engine operates on a pre-compiled binary snapshot (UBX format) that
//! contains all filter rules in an optimized, cache-friendly layout. The hot path
//! does no allocations and uses zero-copy views into the snapshot data.
//!
//! # Modules
//!
//! - `hash`: Murmur3 hash functions for domain and token hashing
//! - `psl`: Public Suffix List for eTLD+1 extraction
//! - `snapshot`: UBX snapshot format and zero-copy loader
//! - `url`: Fast URL parsing without allocations
//! - `matcher`: Core request matching engine
//! - `types`: Shared type definitions

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub mod hash;
pub mod psl;
pub mod snapshot;
pub mod types;
pub mod url;
pub mod matcher;

// Re-export commonly used types
pub use hash::{Hash64, hash64, hash_domain, hash_token};
pub use psl::{get_etld1, is_third_party};
pub use snapshot::Snapshot;
pub use matcher::Matcher;
pub use types::{RequestContext, RuleAction, RequestType, MatchResult, MatchDecision};
