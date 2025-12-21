//! UBX Snapshot Format and Loader
//!
//! This module provides the binary format specification and zero-copy loader
//! for the UBX snapshot format.

mod format;
mod loader;

pub use format::*;
pub use loader::*;
