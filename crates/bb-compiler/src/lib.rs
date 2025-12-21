//! BetterBlocker Filter List Compiler
//!
//! This crate compiles ABP/uBO filter lists into the UBX snapshot format.

pub mod parser;
pub mod optimizer;
pub mod builder;

pub use builder::build_snapshot;
pub use optimizer::optimize_rules;
pub use parser::{parse_filter_list, CompiledRule, DomainConstraint};
