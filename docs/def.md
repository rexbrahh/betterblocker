# BetterBlocker MV2 Semantics

Status: Definitive
This document defines the authoritative matching semantics and precedence rules.

## 1. Terms and request context

For each request, derive:

- req.url: request URL
- req.type: MV2 request type (main_frame, sub_frame, script, image, xhr, fetch, font, media, ping, websocket, other)
- ctx.documentUrl: the document URL into which the resource is loaded (if available)
- ctx.initiator: initiator/origin URL (if available)
- ctx.siteHost: hostname of the top document context for this request
- ctx.reqHost: hostname of the request URL
- ctx.siteETLD1: eTLD+1 for siteHost
- ctx.reqETLD1: eTLD+1 for reqHost
- ctx.isThirdParty: ctx.siteETLD1 != ctx.reqETLD1
- ctx.scheme: scheme of req.url

If multiple context fields exist, determine a consistent priority order per browser:
- Chromium: initiator preferred, then documentUrl
- Firefox: originUrl/documentUrl preferred

This is an adapter detail. Semantics assume ctx.siteHost is correct.

## 2. Rule classes

Network rules:
- ALLOW: `@@...`
- BLOCK: normal network block
- IMPORTANT: network block with `important`
- REDIRECT: block and redirect (`redirect=token`)
- REDIRECT_DIRECTIVE: redirect directive (`redirect-rule=token`)
- REMOVEPARAM: URL sanitize rules
- CSP: document CSP injection
- HEADER_MATCH: response header based block/allow
- RESPONSEHEADER_REMOVE: remove specific response headers (document only)

Cosmetics:
- HIDE selectors
- EXCEPT selectors
- GENERIC selectors
- elemhide / generichide flags
- Procedural programs

Scriptlets:
- Scriptlet invocations
- Scriptlet disable rules

Dynamic:
- Matrix policy rules

## 3. Compile-time preprocessing semantics

### 3.1 badfilter
A rule containing `badfilter` disables the corresponding canonical rule. This is resolved at compile time:
- Build canonical keys for rules
- Remove rules that are badfiltered

No runtime cost.

### 3.2 Normalization
Normalize:
- lowercasing domains
- punycode where required
- option ordering
- domain= lists canonicalization

## 4. Runtime decision pipeline

There are two main runtime stages:
- Stage A: request start (onBeforeRequest)
- Stage B: response headers (onHeadersReceived)

Cosmetics and scriptlets are applied in the renderer using content scripts but are still governed by rule semantics and exceptions.

## 5. Stage A: onBeforeRequest semantics

Evaluation order:

### A0: Trusted-site bypass
If the site is trusted (user override), allow immediately.

### A1: Dynamic filtering matrix
Apply dynamic rules:
- If result is ALLOW: allow immediately
- If result is BLOCK: mark as blocked and proceed to redirect directive resolution
- If NOOP: continue

### A2: removeparam
If any removeparam rules match:
- Compute sanitized URL
- If URL changed, return redirectUrl to sanitized URL
- Apply loop protection:
  - prevent repeated redirect of same URL in same tab/frame

### A3: Static network filtering
Find matching rules for the request:
- IMPORTANT blocks
- ALLOW rules
- BLOCK rules

Precedence:
1) If any IMPORTANT block matches, the request is blocked and all ALLOW exceptions are ignored.
2) Otherwise, if any ALLOW matches, allow.
3) Otherwise, if any BLOCK matches, block.
4) Otherwise, allow.

Matching includes:
- pattern match
- type, party, scheme masks
- domain constraints

### A4: Redirect semantics
Redirect logic applies only when the request is blocked.

- redirect=token:
  - Equivalent to block and redirect
  - If match, return redirectUrl to resource token mapping
- redirect-rule=token:
  - Directive only
  - Only considered if blocked by another rule (block or important or dynamic)

Redirect directive precedence:
1) Gather all matching redirect directives.
2) Apply redirect-rule exceptions:
   - Exceptions disable redirect behavior but do not unblock.
3) Choose highest priority directive (if priority feature is supported).
4) If no directive applies, cancel.

If redirect resource is missing or invalid:
- Do not redirect
- Cancel instead

## 6. Stage B: onHeadersReceived semantics

Only applied when response headers are available.

### B0: Document-only gate
CSP injection and responseheader removal apply only to:
- main_frame
- sub_frame

### B1: responseheader(name) removal
Remove only permitted headers (safe allowlist). Never allow removal of CSP.

### B2: CSP injection
For matching CSP rules:
- Append Content-Security-Policy header value(s)
- Exception behavior:
  - empty $csp exception disables all CSP injections for matching context
  - $csp exception matching specific content disables only that injection

### B3: header= rules
Evaluate response header match rules:
- Determine header-based BLOCK matches
- Determine header-based ALLOW matches

Precedence mirrors network rules:
1) IMPORTANT header-block overrides exceptions.
2) Otherwise ALLOW overrides BLOCK.
3) Otherwise BLOCK cancels.
4) Otherwise no change.

## 7. Cosmetics semantics

### 7.1 Selector application
For a page context:
1) If elemhide applies, inject nothing.
2) Inject site-specific hide selectors minus site-specific exceptions.
3) If generichide applies, skip generic selectors; otherwise inject generic selectors.

### 7.2 Procedural rules
Procedural rules are evaluated:
- after initial DOM availability
- then on DOM mutations via a throttled observer
Constraints:
- bounded work per tick
- avoid unbounded traversal

## 8. Scriptlet semantics

1) If global disable `#@#+js()` applies, inject none.
2) If site disable `domain#@#+js()` applies, inject none for that site.
3) Otherwise inject only hostname-specific scriptlets. No generic scriptlets.

Scriptlets are injected into page context and must be from a vetted library by default.

## 9. Domain constraint semantics (domain=)

Domain constraints are hostname-based, not only eTLD+1.
- Store domain include/exclude sets as hostname hashes.
- At runtime, evaluate constraints against ctx.siteHost by suffix walking:
  - foo.bar.example.com
  - bar.example.com
  - example.com

Constraints are applied during rule verification.

## 10. Regex policy

Regex is supported only as a slow lane with constraints:
- must have a token prefilter hit
- must be bounded in length and complexity
- must use a safe regex engine or impose strict validation

If regex support is not available, skip regex rules and report them in compile stats.

## 11. Determinism and stability

Given identical inputs:
- lists
- config
- PSL data
- compiler version

The snapshot and runtime decisions must be deterministic.
# UBX Snapshot Format

Status: Definitive (v1)
Magic: UBX1
Endianness: little-endian

## 1. Design goals

- Immutable binary blob for fast loading and matching
- Typed-array friendly
- Section directory enables forward compatibility
- Fast membership queries via open-addressing hash tables
- Candidate selection via token posting lists
- Deterministic output for identical inputs (where practical)

## 2. File layout

### 2.1 Header (fixed size)

Fields:
- magic: "UBX1"
- version: u16
- flags: u16
- headerBytes: u32
- sectionCount: u32
- sectionDirOffset: u32
- sectionDirBytes: u32
- buildId: u32
- snapshotCrc32: u32 (optional if flag enabled)

All offsets are from start of file.

### 2.2 Section directory

Each entry:
- id: u16
- flags: u16 (compression reserved, usually 0)
- offset: u32
- length: u32
- uncompressedLength: u32 (0 if uncompressed)
- crc32: u32 (optional)

Unknown sections are ignored.

## 3. Required sections (v1)

Section IDs are stable.

- STRPOOL
- PSL
- DOMAIN_INDEX
- TOKEN_DICT
- TOKEN_POSTINGS
- PATTERN_POOL
- RULES
- DOMAIN_CONSTRAINT_POOL
- REDIRECT_RESOURCES
- REMOVEPARAM_SPECS
- CSP_SPECS
- HEADER_SPECS
- RESPONSEHEADER_RULES
- COSMETIC_RULES
- PROCEDURAL_RULES
- SCRIPTLET_RULES

## 4. STRPOOL

- bytesLen: u32
- bytes: u8[bytesLen] UTF-8

String references are:
- strOff: u32
- strLen: u32

Validation:
- STRPOOL must be valid UTF-8 (validate once on load).

## 5. PSL

PSL is stored as data inside the snapshot. Representation may be:
- hash sets (exact, wildcard, exception) OR
- trie

Requirement:
- Must support eTLD+1 extraction.
- Must be updateable without extension release by distributing new snapshot.

## 6. DOMAIN_INDEX

Maps domainHash -> posting list (ruleIds).

Rationale:
- A single domain may map to multiple rules. Do not store only one ruleId.

Format (recommended):
- open addressing table:
  - domainHash (lo32, hi32)
  - postingsOff: u32
  - ruleCount: u32

Posting list bytes stored in a domain postings blob:
- varint delta-coded sorted ruleIds

## 7. TOKEN_DICT and TOKEN_POSTINGS

TOKEN_DICT maps tokenHash -> postings list:
- tokenHash: u32
- postingsOff: u32
- ruleCount: u32

TOKEN_POSTINGS is a bytes blob of varint delta-coded ruleIds.

Rules:
- tokenHash must never be 0 (reserve 0 for empty slot)
- varint is unsigned LEB128

## 8. PATTERN_POOL

Contains compiled programs for pattern verification.

Recommended structure:
- patternCount: u32
- index table entries:
  - progOff: u32
  - progLen: u16
  - anchorType: u8
  - flags: u8 (case sensitive, right anchor, boundary, etc)
  - hostHashLo: u32 (optional)
  - hostHashHi: u32 (optional)
- progBytesLen: u32
- progBytes: u8[progBytesLen]

Opcode set is implementation-defined but must support:
- literal find
- anchors
- boundary
- hostname anchoring
- DONE

Regex patterns:
- stored as STRPOOL references and flagged as regex
- evaluated in slow lane only

## 9. RULES (SoA tables)

ruleCount: u32

Arrays:
- action: u8[ruleCount]
- flags: u16[ruleCount]
- typeMask: u32[ruleCount]
- partyMask: u8[ruleCount]
- schemeMask: u8[ruleCount]
- patternId: u32[ruleCount] (or 0xFFFFFFFF)
- domainConstraintOff: u32[ruleCount] (or 0xFFFFFFFF)
- optionId: u32[ruleCount] (meaning depends on action)
- priority: i16[ruleCount]
- listId: u16[ruleCount]
- rawTextRef: optional (for logger)

Action IDs (required):
- ALLOW
- BLOCK
- REDIRECT
- REDIRECT_DIRECTIVE
- REMOVEPARAM
- CSP_INJECT
- HEADER_MATCH_BLOCK
- HEADER_MATCH_ALLOW
- RESPONSEHEADER_REMOVE

Flags (required semantics):
- IMPORTANT
- MATCH_CASE
- IS_REGEX
- USER_RULE (optional)
- others reserved

## 10. DOMAIN_CONSTRAINT_POOL

Blob of records.
At domainConstraintOff:
- includeCount: u16
- excludeCount: u16
- include hashes: includeCount * (lo32 u32, hi32 u32)
- exclude hashes: excludeCount * (lo32 u32, hi32 u32)

Runtime:
- Evaluate against ctx.siteHost via suffix walk

## 11. REDIRECT_RESOURCES

Maps resource token to resource path in extension bundle:
- tokenStrRef
- pathStrRef
- mimeKind

Tokens are stable and must match rule option parsing.

## 12. REMOVEPARAM_SPECS

Each spec defines parameter removal:
- mode: u8 (removeAll, literalKey, regexNameValue)
- strRef (literal or regex)
- reserved fields

## 13. CSP_SPECS

Each spec:
- cspStrRef

## 14. HEADER_SPECS

Each spec:
- headerNameStrRef
- matchKind: presence, literal, regex
- invert: bool
- valueStrRef (optional)

## 15. RESPONSEHEADER_RULES

Domain map to header IDs to remove, document-only.
Must restrict removal to safe allowlist headers.

## 16. COSMETIC_RULES

Domain map records:
- hide selectors
- exception selectors
- flags (elemhide, generichide)

Generic selectors:
- cheap generic
- expensive generic (optional)

## 17. PROCEDURAL_RULES

Domain map to procedural programs.

Programs may be stored as:
- STRPOOL program strings, or
- bytecode

## 18. SCRIPTLET_RULES

Domain map records:
- scriptlet token
- args

Plus:
- global disable flag support

## 19. Snapshot validation rules (required)

On load:
- magic/version match
- section offsets and lengths within file bounds
- STRPOOL UTF-8 valid
- optional CRC32 verified
- reject invalid snapshots, retain last known good snapshot

## 20. Versioning

- UBX major version changes only when format breaks.
- Minor additions occur via new sections and flags, old readers ignore unknown sections.
# Packaging and Distribution Readiness

Status: Definitive gate checklist
This document is the authoritative readiness rubric. Packaging and distribution must not proceed until all P0 gates are satisfied.

## 1. Definitions

- P0: must-have for first public distribution
- P1: should-have, can ship after initial release if risk is controlled
- P2: nice-to-have

Each gate includes:
- requirement
- verification method
- status (PASS/FAIL/NA)
- notes and evidence link

## 2. P0 Correctness gates

### 2.1 Core semantics

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| C0.1 | important ignores exceptions for network blocks | conformance tests |  |
| C0.2 | exceptions override blocks when not important | conformance tests |  |
| C0.3 | domain= constraints are hostname-based (suffix match), not only eTLD+1 | conformance + real cases |  |
| C0.4 | badfilter disables target rule at compile time | compiler tests |  |

### 2.2 Redirects

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| C0.5 | redirect= blocks and redirects to a packaged resource | conformance + manual site tests |  |
| C0.6 | redirect-rule applies only after a block | conformance tests |  |
| C0.7 | redirect-rule exceptions disable redirect without unblocking | conformance tests |  |
| C0.8 | missing redirect resource fails safe (cancel, no loop) | conformance tests |  |

### 2.3 removeparam

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| C0.9 | removeparam literal works (navigation + subrequests where applicable) | conformance + trace replay |  |
| C0.10 | loop protection prevents infinite redirects | conformance tests |  |
| C0.11 | removeparam exceptions disable matching removeparam rules | conformance tests |  |

### 2.4 CSP and header rules

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| C0.12 | $csp injected only for main_frame/sub_frame | conformance tests |  |
| C0.13 | empty $csp exception disables all CSP injections | conformance tests |  |
| C0.14 | $csp exception matching content disables only that injection | conformance tests |  |
| C0.15 | header= matching and inversion works | conformance tests |  |
| C0.16 | header exceptions override header blocks unless important | conformance tests |  |
| C0.17 | responseheader removes only safe allowlist headers, document-only | conformance tests |  |

### 2.5 Cosmetics and scriptlets

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| C0.18 | elemhide disables all cosmetics | conformance + manual |  |
| C0.19 | generichide disables generic cosmetics only | conformance + manual |  |
| C0.20 | selector exceptions apply correctly | conformance tests |  |
| C0.21 | scriptlets injected at document_start into page context | manual + integration tests |  |
| C0.22 | global and per-site scriptlet disable rules work | conformance tests |  |
| C0.23 | no generic scriptlet injection | conformance tests |  |

### 2.6 Dynamic filtering and UI plumbing

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| C0.24 | dynamic matrix allow/block/noop precedence correct | conformance tests |  |
| C0.25 | trusted-site bypass works | manual |  |
| C0.26 | decisions are explainable via logger (rule + list) | manual + tests |  |

## 3. P0 Performance gates

### 3.1 Hot path constraints

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| P0.1 | steady-state hot path alloc-free (no per-request Vec/String growth) | profiling + counters |  |
| P0.2 | no per-match full URL lowercasing | code audit + perf |  |
| P0.3 | regex is slow lane only and prefiltered | code audit + tests |  |
| P0.4 | caches exist and improve tail latency | perf tests |  |

### 3.2 Benchmarks

Required benchmarks:
- Synthetic workload benchmark (batch timing)
- Trace replay benchmark (JSONL)
- In-browser MV2 instrumentation benchmark

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| P0.5 | matcher p99 under synthetic workload is stable across runs | bench results |  |
| P0.6 | match_request overhead bounded and does not dominate | bench results |  |
| P0.7 | MV2 handler end-to-end p99 < 5ms on representative sites | in-browser telemetry |  |

## 4. P0 Security gates

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| S0.1 | snapshot validation rejects malformed blobs safely | fuzz + unit tests |  |
| S0.2 | compiler caps list sizes and rules processed | tests |  |
| S0.3 | compile report includes skipped rule counts by reason | UI + tests |  |
| S0.4 | web_accessible_resources minimized to required assets | manifest audit |  |
| S0.5 | scriptlet library is vetted and fixed by default | review |  |
| S0.6 | options UI and other pages are XSS-safe | security review |  |

## 5. P0 Reliability gates

| Gate | Requirement | Verification | Status |
|---|---|---|---|
| R0.1 | atomic snapshot swap (validate then switch) | tests |  |
| R0.2 | rollback to last known good snapshot works | tests |  |
| R0.3 | update pipeline cannot stall request handling | stress test |  |
| R0.4 | fail-open on internal errors by default | tests |  |
| R0.5 | persistent state survives browser restart | integration tests |  |

## 6. Conformance test inventory (minimum required)

- important vs exception
- redirect vs redirect-rule exception does not unblock
- $csp injection and its exception behaviors
- header matching with inversion and exception override
- responseheader removal restrictions
- elemhide and generichide behavior
- scriptlet injection and disable rules
- domain scoping with hostnames and suffix match
- removeparam correctness and loop protection

All tests must run in CI and locally.

## 7. Distribution packaging checklist

| Item | Requirement | Status |
|---|---|---|
| D0.1 | versioning, changelog, and migration notes |  |
| D0.2 | default list set defined with pinned versions |  |
| D0.3 | update strategy defined (auto, manual, frequency) |  |
| D0.4 | user-visible compile stats and errors |  |
| D0.5 | export logs and traces for bug reports |  |
| D0.6 | license compliance for included lists/resources |  |
| D0.7 | crash and performance telemetry policy defined (if any) |  |

## 8. Release decision rule

BetterBlocker is ready for packaging and distribution only when:
- All P0 gates are PASS
- No known correctness bugs exist in core semantics
- In-browser MV2 handler p99 meets target under representative browsing traces

P1 and P2 items may ship later, but only if they do not compromise safety or correctness.
