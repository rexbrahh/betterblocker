# BetterBlocker

BetterBlocker is a high-performance MV2 content blocker that aims to match the capabilities of uBlock Origin while providing a tight, low-allocation matching engine. It targets a Rust-based core compiled to WebAssembly to provide efficient network filtering, cosmetic filtering, and scriptlet injection.

## Overview

The project is built around the **UBX snapshot approach**. Raw filter lists are compiled into a compact, immutable binary format (UBX) optimized for runtime matching. This aims to allow for extremely fast decision latency and predictable performance even under heavy subresource loads.

### Target Features

*   **Network Filtering**: Planned support for ABP/uBO syntax including basic patterns, exceptions (`@@`), and options (resource types, third-party, domain scoping, `important`, etc.).
*   **Redirects and Surrogates**: Aims to support `$redirect` and `$redirect-rule` for blocking and redirecting to a fixed resource library.
*   **CSP Injection**: `$csp` rules for injecting additional Content Security Policies.
*   **Header-aware Filtering**: Matching and removal of response headers.
*   **Cosmetic Filtering**: Both declarative (CSS selectors) and procedural cosmetics.
*   **Scriptlets**: Page-context JS defenses injected at `document_start`.
*   **Dynamic Filtering**: uBO-like matrix for per-site and global overrides.
*   **Provenance Logging**: A logger to explain "why was this blocked?" including rule and list provenance.

## Architecture

*   **bb-core (Rust)**: The performance-critical matching engine and UBX loader.
*   **bb-compiler (Rust)**: The pipeline that ingests filter lists and emits optimized UBX snapshots.
*   **bb-cli (Rust)**: Command-line tool for snapshot management, compilation, and validation.
*   **bb-wasm (Rust)**: WebAssembly bindings for the matching engine, used by the browser extension.
*   **Extension (TypeScript)**: The MV2 extension glue that registers `webRequest` listeners and manages the UI.

## Repository Layout

```text
crates/
  bb-cli/       - CLI tool for snapshot management
  bb-compiler/  - Filter list compiler
  bb-core/      - Core matching engine
  bb-wasm/      - WebAssembly bindings
extension/      - Static extension assets and manifest (MV2)
src/
  bg/           - Background script source
  cs/           - Content script source
  options/      - Options page source
  popup/        - Popup source
  shared/       - Shared TypeScript utilities
bench/          - Performance benchmarking scripts
tests/          - TypeScript unit and E2E tests
```

## Build & Development Workflow

The project uses `bun` for the TypeScript/JS workflow and `cargo` for the Rust components.

### Prerequisites

*   [Bun](https://bun.sh/)
*   [Rust](https://www.rust-lang.org/)
*   [wasm-pack](https://rustwasm.github.io/wasm-pack/)

### Commands

*   **Build WASM**: `bun run build:wasm`
*   **Compile Extension**: `bun run build`
*   **Development Build**: `bun run build:dev`
*   **Typecheck**: `bun run typecheck`
*   **Create Distribution**: `bun run dist` (Builds and copies assets to `dist/`)
*   **Watch Mode**: `bun run watch`
*   **Run TS Tests**: `bun run test`
*   **Run Rust Tests**: `cargo test --all`
*   **E2E Tests**: `bun run test:e2e`
*   **Compile Snapshot**: `bun run compile` (Runs `bb-cli` to compile filter lists)

## Benchmarks & Performance

BetterBlocker aims for a per-request decision latency of **< 5 ms p99** in the real MV2 `onBeforeRequest` handler.

*   **Run Benchmarks**: `bun run bench`
*   **Check Performance Budget**: `bun run perf-budget`

Current measured matcher performance (on modern CPUs):
*   `should_block`: ~1.3 us avg
*   `match_request`: ~1.8 us avg

## Release & Downloads

Releases are automatically built and deployed via GitHub Actions.

*   **GitHub Pages**: [https://rexbrahh.github.io/betterblocker/](https://rexbrahh.github.io/betterblocker/)
*   **Downloads**: The following artifacts are available on the Pages site:
    *   `betterblocker.zip` (For manual developer mode installation)
    *   `betterblocker.crx` (For enterprise policy installation)

## Status & License

**Status**: Development / Pre-alpha. While the Rust-based core and compiler are largely functional, the **extension integration is currently incomplete** (matching engine is not yet wired into the background service). Several uBO-class features (removeparam, CSP/header phases, cosmetics, scriptlets) are also in various stages of implementation. It is not yet ready for general use.

**License**: MIT
