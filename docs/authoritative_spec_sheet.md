# BetterBlocker MV2 Spec Sheet

**Document status:** definitive design + readiness gate checklist
**Scope:** Rust-based MV2 content blocker (Chromium MV2 forks + Firefox MV2), uBO-class semantics and features, with a compiled snapshot engine and high-performance runtime.

---

## 1) Product definition

**BetterBlocker** is a high-performance MV2 content blocker that aims to match uBlock Origin’s capabilities for:

* Network filtering (block, allow, exceptions, dynamic rules)
* Redirect resources (surrogates) and redirect directives
* CSP injection
* Header-aware filtering (response-header matching, and response header removal via rules)
* Cosmetic filtering (CSS selectors and procedural)
* Scriptlets (page-context JS defenses)
* Logging and “why was this blocked?” provenance
* Element picker / zapper for rule authoring

**Core strategy**

* Compile raw lists into a compact, immutable **UBX snapshot** optimized for runtime matching.
* Runtime matching is a tight, low-allocation engine using indexed candidate selection (token buckets) plus fast verification.
* MV2 webRequest JS glue is unavoidable, but can be reduced to a minimal syscall-style wrapper around the matcher.

---

## 2) Goals and non-goals

### 2.1 Goals

**Correctness**

* Match uBO filter semantics for supported features, including precedence rules (exceptions vs important, redirect vs redirect-rule exceptions, domain scoping, resource type scoping).

**Performance**

* Per-request decision latency target: **< 5 ms p99** in the real MV2 `onBeforeRequest` handler.
* Design target: microseconds for matcher itself, with predictable tail latency under heavy subresource loads.
* Scale target: handle **500k+ input rules** (list size), with aggressive compile-time normalization and index construction.

**Reliability**

* Snapshot updates are atomic and rollback-safe.
* Requests are never blocked due to internal failures (fail-open unless user explicitly chooses fail-closed modes).

**Security**

* Treat filter lists as untrusted input. Snapshot build must be robust against malformed or adversarial rules.

### 2.2 Explicit non-goals

* Server-side tracking prevention (cannot stop what the origin server does).
* Response body rewriting (not supported on Chromium MV2).
* Request body rewriting (POST payload rewriting).

---

## 3) Target environments

### 3.1 Primary target

* MV2-capable Chromium forks (ungoogled-chromium based, etc).
* Firefox MV2.

### 3.2 Secondary future directions

* MV3 support later via compiled declarative rulesets (DNR), with reduced semantics.
* Browser-integrated native blocker (Helium option 1) or native declarative API (Helium option 2) for “no JS on hot path” and stronger anti-fingerprinting.

---

## 4) Feature scope and requirements (uBO parity targets)

### 4.1 Network filtering (required)

Support ABP/uBO network rule syntax:

* Basic patterns: `||`, `|`, `*`, `^`, substring
* Exceptions: `@@`
* Options:

  * resource type filters (`script`, `image`, `stylesheet`, `xhr/fetch`, `font`, `media`, `frame`, `websocket`, `ping`, etc)
  * first-party vs third-party (derived from initiator vs destination)
  * scheme filters (`http`, `https`, `ws`, `wss`)
  * domain scoping (`domain=` include/exclude lists)
  * `important`
  * `badfilter` (compile-time removal)

### 4.2 Redirects and surrogates (required)

* `$redirect=token` “block and redirect” with a fixed resource library.
* `$redirect-rule=token` redirect directives that apply only when a request is blocked.
* Exceptions for redirect directives (`@@...$redirect-rule=...`) disable redirect behavior without unblocking.

### 4.3 URL parameter removal (required)

* `$removeparam=...` rules:

  * literal parameter names (fast path)
  * optional regex forms (slow lane, safe-regex constraints)
* Loop protection on redirect-to-sanitized-URL.

### 4.4 CSP injection (required)

* `$csp=...` adds additional CSP to document responses only (`main_frame`, `sub_frame`).
* Exceptions:

  * exception matching specific CSP content disables only that CSP injection
  * empty `$csp` exception disables all CSP injections for that page scope

### 4.5 Header-aware rules (required)

* Response header matching rules (`$header=` style):

  * match header presence or value (literal or regex)
  * invert match with `~`
* `responseheader()` removal rules:

  * document-only
  * limited safe header set (avoid allowing removal of CSP)

### 4.6 Cosmetics (required)

* Declarative cosmetics:

  * `domain##selector`
  * exceptions `domain#@#selector`
  * `elemhide` to disable all cosmetics on a site
  * `generichide` to disable generic cosmetics on a site
* Procedural cosmetics:

  * JS-driven evaluation with throttled mutation observers and strict work budgets

### 4.7 Scriptlets (required)

* `domain##+js(scriptlet, args...)`
* Exceptions:

  * `domain#@#+js()` disables on site
  * `#@#+js()` disables globally
* Scriptlets must be injected into **page context** (not isolated world) to patch page JS.

### 4.8 Dynamic filtering (required)

* uBO-like matrix:

  * per-site and global allow/block overrides by type and 1p/3p
* Runs before static filters.

### 4.9 Logging and tooling (required for distribution readiness)

* Logger with provenance:

  * what rule matched, from which list, why it won precedence
* Export traces for benchmarking and bug reports.
* Element picker / zapper to generate cosmetic rules and exceptions.

---

## 5) Architecture

### 5.1 Components

1. **Compiler**

* Ingest lists, parse, normalize, dedupe, optimize.
* Emit UBX snapshot.
* Supports:

  * CLI compilation (developer tooling)
  * Optional in-browser worker compilation (self-updates)

2. **Runtime matcher**

* Loads UBX snapshot into a read-only view.
* Implements the decision pipeline for:

  * request start (block/allow/redirect/removeparam)
  * response headers (CSP injection, header-aware filtering, responseheader removal)
* Maintains caches and scratch buffers for allocation-free matching.

3. **MV2 extension**

* JS glue registers webRequest listeners and delegates to matcher.
* Content scripts implement cosmetics and scriptlets injection.
* UI for settings, list management, logger, picker.

### 5.2 JS vs Wasm strategy (definitive)

* **JS glue is mandatory** for MV2 APIs.
* Preferred runtime plan:

  * Keep JS glue minimal.
  * Matcher can be in wasm if boundary costs are controlled.
  * Long-term: avoid per-request JS object creation; prefer packed return values and minimal string passing.

Current implementation uses a wasm matcher called from the MV2 background handler .

---

## 6) UBX snapshot format (UBX1)

### 6.1 General principles

* One immutable binary blob (ArrayBuffer).
* Section directory for forward-compatible additions.
* Typed-array friendly layout.
* Varint delta-coded postings lists.

### 6.2 Required sections (target spec)

* STRPOOL: interned UTF-8 strings
* PSL: compiled public suffix rules (data, not baked code)
* DOMAIN_INDEX: domainHash -> postings list
* TOKEN_DICT + TOKEN_POSTINGS: tokenHash -> postings list
* PATTERN_POOL: compiled pattern programs (bytecode)
* RULES: SoA tables (action, flags, masks, ids)
* DOMAIN_CONSTRAINT_POOL: include/exclude hostname hashes
* REDIRECT_RESOURCES: token -> resource path + metadata
* REMOVEPARAM_SPECS
* CSP_SPECS
* HEADER_SPECS
* RESPONSEHEADER_RULES
* COSMETIC_RULES (domain and generic)
* PROCEDURAL_RULES
* SCRIPTLET_RULES

### 6.3 Runtime invariants

* Snapshot must be validated on load:

  * magic/version
  * section bounds
  * optional CRC32
  * STRPOOL UTF-8 validity
* Unknown sections are ignored.

---

## 7) Matching semantics (definitive precedence rules)

### 7.1 Request stage: onBeforeRequest

Evaluation order:

1. Trusted-site bypass (if user enabled)
2. Dynamic filtering matrix (allow/block/noop)
3. removeparam modifications (redirect to sanitized URL)
4. Static filtering:

   * IMPORTANT blocks win and ignore exceptions
   * Otherwise: exceptions override blocks
5. Redirect directives:

   * Only applied if the request is blocked
   * redirect-rule exceptions disable redirect behavior only
   * highest priority directive wins
6. Return:

   * allow
   * cancel
   * redirectUrl

### 7.2 Response stage: onHeadersReceived

Document-only gate for CSP and responseheader:

1. responseheader() removal (safe limited set)
2. CSP injection ($csp), with exception semantics
3. header-based blocking/unblocking:

   * IMPORTANT header-block ignores exceptions
   * otherwise exception overrides block
4. Return modified headers or cancel

### 7.3 Cosmetics and scriptlets

* Cosmetics:

  * `elemhide` disables all
  * `generichide` disables generic only
  * per-domain selectors minus exceptions
* Scriptlets:

  * global and per-site disable rules win
  * no generic scriptlets
  * inject at document_start into page context

---

## 8) Performance model and budgets

### 8.1 Where the time goes in MV2

Total cost per request in practice:

* webRequest dispatch overhead
* JS glue (field extraction)
* matcher boundary (JS to wasm if used)
* matcher work (index lookups + verification)
* result marshalling (avoid object creation if possible)

### 8.2 Design requirements for <5 ms p99

* No per-request allocations on the hot path.
* No full-URL lowercasing per candidate match.
* Tokenization is bounded and allocation-free.
* Regex is a strict slow lane and must be prefiltered.
* Scratch buffers reused across calls.

### 8.3 Current measured matcher performance (bench harness)

Using the “realistic benchmark” harness:

* `should_block`: ~1.3 µs avg, ~1.5 µs p99
* `match_request`: ~1.8 µs avg, ~2.1 µs p99

These are promising for the matcher itself, but packaging readiness requires measuring inside the actual MV2 handler in a real browser runtime.

---

## 9) Security model

### 9.1 Threats

* Malicious filter list content:

  * memory blowups (huge lines, huge counts)
  * pathological patterns (worst-case matching)
* Extension surface:

  * options pages and UIs (XSS risks)
  * web_accessible_resources (exposure and fingerprinting surface)
* Scriptlet injection:

  * page-context injection is powerful and must be controlled

### 9.2 Mitigations (required for distribution readiness)

* Compiler limits:

  * maximum list size and maximum rules processed
  * bounded regex support with safe-regex checks
  * compile report: counts skipped by reason
* Snapshot validation:

  * strict bounds checking
  * CRC32 optional but recommended
* Web accessible resources:

  * expose only what must be exposed for redirects and page-context injection
* Scriptlet safety:

  * only vetted built-in scriptlets by default
  * user scriptlets are off by default or require explicit trust

---

## 10) Testing and conformance

### 10.1 Conformance test suite (required)

Synthetic, deterministic tests for:

* important vs exception
* redirect and redirect-rule exception behavior
* CSP injection and `$csp` exception semantics
* header-based matching and exception override
* responseheader limitations
* scriptlet disable rules
* domain scoping correctness (hostname, not just eTLD+1)

### 10.2 Integration regression

* uBO built-in lists (unbreak, filters, resources) to exercise uBO-specific features
* EasyList + EasyPrivacy for ABP ecosystem baseline
* HaGeZi/OISD for performance and scale stress

### 10.3 Real-trace benchmarking (required for readiness)

* MV2 trace recorder in extension background
* replay traces through bench harness
* browser-internal timing: measure handler entry to return BlockingResponse

---

## 11) Packaging and distribution readiness checklist (gate criteria)

### 11.1 P0 correctness gates (must be true)

* Redirect semantics are correct end-to-end:

  * `$redirect=` blocks and redirects properly
  * `$redirect-rule=` applies only to blocked requests
  * redirect-rule exceptions disable redirect without unblocking
* Domain rule storage cannot drop rules (domainHash must map to postings, not single ruleId).
* Domain constraints use hostname suffix matching, not only eTLD+1.
* removeparam works with loop protection.
* CSP injection and its exception semantics are implemented and tested.
* Header-based rules and responseheader removal implemented with safe constraints.
* Cosmetics and scriptlets implement the required disable semantics.

### 11.2 P0 performance gates (must be true)

* Hot path is allocation-free in steady state.
* No full-URL lowercasing per match attempt.
* Trace-based p99 in actual MV2 handler meets target (set an initial internal target like <200 µs p99, then tighten).

### 11.3 P0 security gates (must be true)

* List inputs capped and validated.
* Snapshot validation is strict.
* web_accessible_resources minimized.
* Scriptlet library is vetted and cannot be trivially abused by a hostile list.

### 11.4 Operational gates (must be true)

* Atomic snapshot swap with rollback.
* Update mechanism is robust and non-blocking.
* Logger works and can explain decisions.

---

## 12) Current implementation status vs this spec (honest delta)

Based on the current repo review:

* Network matching core exists and is fast in microbenchmarks.
* The MV2 handler calls wasm matching already .
* Several uBO-class features are not fully implemented yet (removeparam, CSP/header phases, cosmetics, scriptlets).
* There are known P0 correctness issues to fix before claiming uBO parity (redirect representation, domain rule storage dropping rules, domain scoping semantics).

This means: **not ready for packaging and distribution as “uBO-class” yet**, even though the matcher performance looks excellent.

---

## 13) Future direction: Helium integration (optional)

If you later integrate into Helium:

* Option 1 (built-in blocker in browser) unlocks stronger anti-fingerprinting and eliminates MV2 JS hot-path overhead.
* Option 2 (native declarative API) keeps extensions as UI/config but moves request matching into C++ native path.

This is a separate track from MV2 extension distribution.

---
