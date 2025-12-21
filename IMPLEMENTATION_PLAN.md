# BetterBlocker Implementation Plan

This document tracks the technical specifications, current status, and roadmap for the BetterBlocker project.

## Locked Decisions & Specifications

### UBX Snapshot Format (v1)
A zero-copy binary format designed for extremely fast loading and matching in both Rust and WebAssembly environments.

- **Magic Bytes**: `UBX1` (`[0x55, 0x42, 0x58, 0x31]`)
- **Endianness**: Little-endian for all values.
- **Alignment**: 4-byte or 8-byte alignment for section data to ensure safe zero-copy mapping.
- **Hashing**:
    - **Hash64**: Composite key using two Murmur3 32-bit passes.
    - **Seed LO**: `0x9e3779b9` (Golden ratio)
    - **Seed HI**: `0x85ebca6b` (Murmur3 constant)
    - **Sentinel**: `(0, 0)` is reserved for empty slots. Results are OR-ed with `lo |= 1` to avoid the sentinel.

### Matching Precedence
Rules are evaluated in a specific order to ensure consistent behavior across different list maintainers.

1. **Important Block**: Rules with `important` flag override everything else, including exceptions.
2. **Allow (Exception)**: Rules that allow a request override normal block rules.
3. **Normal Block**: Standard blocking rules.
4. **Redirect**: Can be combined with block rules (normal or important). If a match is found, the redirect resource is applied.

### Platform & Runtime Decisions
- **Target**: MV2 for Chromium forks and Firefox MV2.
- **Core Engine**: Rust implementation with optional WASM integration for extension use.
- **Hot Path**: Favor JS-only interception glue; use WASM for matching once profiling shows it is a net win.
- **PSL**: Stored as snapshot data (not embedded in code).
- **Compiler Frontends**: Single Rust compiler crate with CLI and WASM worker frontends.

### Component Architecture
- **bb-core (Rust)**: Performance-critical matching engine and UBX loader.
- **bb-compiler (Rust)**: Pipeline to convert ABP/uBO filter lists into optimized UBX snapshots.
- **bb-cli (Rust)**: Command-line tool for snapshot management and testing.
- **bb-wasm (Rust)**: WASM bindings for the matching engine to be used in the web extension.
- **Extension (TS)**: Browser extension wrapper for MV2 using blocking webRequest APIs.

---

## Current Status

| Component | Status | Description |
|-----------|--------|-------------|
| bb-core | Complete | Matcher, UBX loader, PSL, URL utils, and hashing implemented. |
| bb-compiler | Complete | Parser, option masks, optimizer, and UBX builder with domain sets, rules, and domain constraints. |
| bb-cli | Complete | CLI with compile/validate/info commands fully wired. |
| bb-wasm | Complete | Full WASM bindings with match_request, should_block, and utility exports (92KB binary). |
| extension | Started | MV2 manifest + background utilities present; matching not integrated. |

---

## Milestones

- [x] **Milestone 1: Core Engine**
    - [x] Define UBX v1 binary format
    - [x] Implement zero-copy loader
    - [x] Implement Murmur3-based Hash64
    - [x] Implement core matching logic with precedence
    - [x] URL tokenization and host suffix walking

- [x] **Milestone 2: Compiler Pipeline**
    - [x] Basic UBX builder (STRPOOL, DOMAIN_SETS, RULES, DomainConstraintPool)
    - [x] Rule parser for host anchors and hosts-file entries
    - [x] Option parsing for type/party/scheme and important
    - [x] Optimizer (rule deduplication with full key)
    - [x] $domain constraints and DomainConstraintPool emission
    - [x] CLI compile/validate/info commands
    - [x] Token dictionary/postings and pattern pool emission (for URL pattern rules)
    - [ ] Redirect resource interning

- [ ] **Milestone 3: WASM & Integration**
    - [x] Full WASM bindings for the matcher
    - [x] TypeScript wrapper for WASM module (wasm-pack generates JS glue)
    - [ ] Extension background service integration
    - [ ] Simple UI for status and list management

---

## Task Breakdown

### bb-compiler (Rust)
- [x] UBX Section Builder (STRPOOL, DOMAIN_SETS, RULES)
- [x] Host-anchored rule parser
- [x] Option parsing for type/party/scheme/important
- [x] Rule deduplication optimizer
- [x] Support for $domain constraints + DomainConstraintPool
- [x] CLI compile/validate/info commands
- [x] Token dictionary/postings + pattern pool
- [ ] Redirect resource mapping

### bb-wasm (Rust)
- [x] Export `Matcher` and `Snapshot` to JS
- [x] Implement `match_request` bridge
- [x] Memory management for snapshots in JS (Box::leak for 'static lifetime)
- [ ] Benchmarking harness for WASM vs JS

### Web Extension (TypeScript)
- [x] URL utility port from Rust
- [ ] WASM loading in background worker
- [ ] Request interception hook (MV2 webRequest)
- [ ] Simple popup for block count
- [ ] Options page for list management

---

## Test & Validation Gates

- **Unit Tests**: Rust crates must maintain >80% coverage.
- **Integration Tests**: CLI must be able to compile a 50k rule list and match accurately against known samples.
- **Performance Gate**: Matcher must process typical URLs in <50 microseconds on modern CPUs.
- **Integrity**: UBX snapshots must pass CRC32 verification and version checks.

---

## Risks & Assumptions

- **WASM Performance**: The overhead of crossing the JS/WASM boundary for every request must be minimal.
- **Memory Usage**: Snapshots must be small enough to stay in memory (aiming for <5MB for 100k rules).
- **Rule Support**: Not all uBO features (procedural filters, scriptlets) may be supported in v1.
