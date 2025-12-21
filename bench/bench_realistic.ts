// bench/bench_realistic.ts
//
// Realistic benchmark harness for BetterBlocker.
// Key differences vs the current bench:
// - Measures wall time over batches (no per-call performance.now noise).
// - Benchmarks both should_block (core) and match_request (extension-facing API).
// - Supports trace playback (jsonl) so you can benchmark real browsing traffic.
// - Generates a more realistic synthetic workload when no trace is provided.

import { $ } from "bun";
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname } from "node:path";

const WASM_PATH = "./dist/wasm/bb_wasm.js";
const DEFAULT_SNAPSHOT_PATH = "./dist/data/snapshot.ubx";
const DEFAULT_FILTER_LIST_PATH = "./testdata/test-filters.txt";

type RequestType =
  | "main_frame"
  | "sub_frame"
  | "script"
  | "stylesheet"
  | "image"
  | "xmlhttprequest"
  | "font"
  | "ping"
  | "media"
  | "websocket"
  | "other";

interface BenchRequest {
  url: string;
  type: RequestType;
  initiator?: string;
  tabId: number;
  frameId: number;
  requestId: string;
}

interface WasmModule {
  init(data: Uint8Array): void;
  is_initialized(): boolean;
  should_block(
    url: string,
    requestType: string,
    initiator: string | undefined,
  ): boolean;
  match_request(
    url: string,
    requestType: string,
    initiator: string | undefined,
    tabId: number,
    frameId: number,
    requestId: string,
  ): { decision: number; ruleId: number; listId: number; redirectUrl?: string };
  get_snapshot_info(): { size: number; initialized: boolean };
}

interface BenchOptions {
  inputPath: string;
  snapshotPath: string;
  compile: boolean;

  mode: "should_block" | "match_request" | "both";
  iterations: number;
  warmupOps: number;

  sampleBatchOps: number;

  tracePath?: string;
  traceLimit: number;

  syntheticPages: number;
  syntheticReqsPerPage: number;
  seed: number;
}

interface BenchResult {
  name: string;
  opCount: number;
  totalMs: number;
  avgUs: number;
  p50Us: number;
  p95Us: number;
  p99Us: number;
  opsPerSec: number;
  blockedPct: number;
}

// ----------------------- util: args -----------------------

function assertFileExists(path: string, hint: string): void {
  if (!existsSync(path)) throw new Error(`${hint} not found at ${path}`);
}

function parseArgs(argv: string[]): BenchOptions {
  let inputPath = process.env.BENCH_INPUT || DEFAULT_FILTER_LIST_PATH;
  let snapshotPath = process.env.BENCH_SNAPSHOT || DEFAULT_SNAPSHOT_PATH;
  let compile = true;

  let mode: BenchOptions["mode"] = "both";
  let iterations = 200;
  let warmupOps = 200_000;
  let sampleBatchOps = 512;

  let tracePath: string | undefined;
  let traceLimit = 50_000;

  let syntheticPages = 60;
  let syntheticReqsPerPage = 120;
  let seed = 0xc0ffee;

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];

    if (arg === "--no-compile") {
      compile = false;
    } else if (arg === "--input" && argv[i + 1]) {
      inputPath = argv[++i]!;
    } else if (arg === "--snapshot" && argv[i + 1]) {
      snapshotPath = argv[++i]!;
    } else if (arg === "--mode" && argv[i + 1]) {
      const v = argv[++i]!;
      if (v === "should_block" || v === "match_request" || v === "both")
        mode = v;
      else throw new Error(`Invalid --mode ${v}`);
    } else if (arg === "--iterations" && argv[i + 1]) {
      iterations = Number(argv[++i]!);
    } else if (arg === "--warmup-ops" && argv[i + 1]) {
      warmupOps = Number(argv[++i]!);
    } else if (arg === "--sample-batch-ops" && argv[i + 1]) {
      sampleBatchOps = Number(argv[++i]!);
    } else if (arg === "--trace" && argv[i + 1]) {
      tracePath = argv[++i]!;
    } else if (arg === "--trace-limit" && argv[i + 1]) {
      traceLimit = Number(argv[++i]!);
    } else if (arg === "--pages" && argv[i + 1]) {
      syntheticPages = Number(argv[++i]!);
    } else if (arg === "--reqs-per-page" && argv[i + 1]) {
      syntheticReqsPerPage = Number(argv[++i]!);
    } else if (arg === "--seed" && argv[i + 1]) {
      seed = Number(argv[++i]!);
    }
  }

  return {
    inputPath,
    snapshotPath,
    compile,
    mode,
    iterations,
    warmupOps,
    sampleBatchOps,
    tracePath,
    traceLimit,
    syntheticPages,
    syntheticReqsPerPage,
    seed,
  };
}

// ----------------------- util: timing -----------------------

function nowNs(): bigint {
  // Bun supports process.hrtime.bigint(), use it for stable wall time.
  if (
    typeof process !== "undefined" &&
    process.hrtime &&
    typeof process.hrtime.bigint === "function"
  ) {
    return process.hrtime.bigint();
  }
  // Fallback
  return BigInt(Math.floor(performance.now() * 1e6));
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.min(
    sorted.length - 1,
    Math.max(0, Math.ceil(sorted.length * p) - 1),
  );
  return sorted[idx]!;
}

// ----------------------- build/load -----------------------

async function compileSnapshot(
  inputPath: string,
  snapshotPath: string,
): Promise<void> {
  assertFileExists(inputPath, "Filter list");
  await mkdir(dirname(snapshotPath), { recursive: true });

  console.log("Compiling filter list to snapshot...");
  await $`cargo run --release --package bb-cli -- compile -i ${inputPath} -o ${snapshotPath} -v`.quiet();
  console.log("Snapshot compiled.");
}

async function loadWasm(): Promise<WasmModule> {
  const wasmJsPath = Bun.resolveSync(WASM_PATH, process.cwd());
  const wasmBinaryPath = wasmJsPath.replace(".js", "_bg.wasm");

  assertFileExists(wasmJsPath, "WASM JS module");
  assertFileExists(wasmBinaryPath, "WASM binary");

  const jsModule = await import(wasmJsPath);
  const wasmBytes = await Bun.file(wasmBinaryPath).arrayBuffer();

  await jsModule.default({ module_or_path: wasmBytes });
  return jsModule as unknown as WasmModule;
}

async function loadSnapshot(snapshotPath: string): Promise<Uint8Array> {
  assertFileExists(snapshotPath, "Snapshot file");
  const buffer = await Bun.file(snapshotPath).arrayBuffer();
  return new Uint8Array(buffer);
}

// ----------------------- trace loader -----------------------
//
// Expected jsonl format (one object per line):
// {"url":"https://...","type":"script","initiator":"https://site/","tabId":1,"frameId":0,"requestId":"123"}
//
// Any missing fields are filled with defaults.

async function loadTraceJsonl(
  path: string,
  limit: number,
): Promise<BenchRequest[]> {
  assertFileExists(path, "Trace file");
  const text = await Bun.file(path).text();
  const lines = text.split("\n");

  const out: BenchRequest[] = [];
  let n = 0;

  for (const line of lines) {
    const s = line.trim();
    if (!s) continue;

    let obj: any;
    try {
      obj = JSON.parse(s);
    } catch {
      continue;
    }

    const url = String(obj.url || "");
    if (!url) continue;

    const type = (String(obj.type || "other") as RequestType) || "other";
    const initiator = obj.initiator ? String(obj.initiator) : undefined;

    const tabId = Number.isFinite(obj.tabId) ? Number(obj.tabId) : 1;
    const frameId = Number.isFinite(obj.frameId) ? Number(obj.frameId) : 0;
    const requestId = obj.requestId ? String(obj.requestId) : String(n);

    out.push({ url, type, initiator, tabId, frameId, requestId });

    n++;
    if (n >= limit) break;
  }

  if (out.length === 0) throw new Error(`Trace loaded 0 requests from ${path}`);
  return out;
}

// ----------------------- synthetic workload -----------------------

function createRng(seed: number): () => number {
  let x = seed >>> 0;
  return () => {
    // xorshift32
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    return (x >>> 0) / 0x100000000;
  };
}

function pick<T>(arr: readonly T[], r: () => number): T {
  return arr[Math.floor(r() * arr.length)]!;
}

function weightedPick<T>(
  items: readonly { item: T; w: number }[],
  r: () => number,
): T {
  let sum = 0;
  for (const it of items) sum += it.w;
  let x = r() * sum;
  for (const it of items) {
    x -= it.w;
    if (x <= 0) return it.item;
  }
  return items[items.length - 1]!.item;
}

function randInt(r: () => number, min: number, max: number): number {
  return Math.floor(r() * (max - min + 1)) + min;
}

function randHex(r: () => number, len: number): string {
  const chars = "0123456789abcdef";
  let s = "";
  for (let i = 0; i < len; i++) s += chars[Math.floor(r() * chars.length)]!;
  return s;
}

function randAlnum(r: () => number, len: number): string {
  const chars = "abcdefghijklmnopqrstuvwxyz0123456789";
  let s = "";
  for (let i = 0; i < len; i++) s += chars[Math.floor(r() * chars.length)]!;
  return s;
}

const TOP_SITES = [
  "google.com",
  "youtube.com",
  "github.com",
  "reddit.com",
  "amazon.com",
  "wikipedia.org",
  "mozilla.org",
  "nytimes.com",
  "cnn.com",
  "twitter.com",
  "x.com",
  "facebook.com",
  "instagram.com",
  "linkedin.com",
  "stackoverflow.com",
  "cloudflare.com",
];

const CDN_DOMAINS = [
  "cdnjs.cloudflare.com",
  "cdn.jsdelivr.net",
  "unpkg.com",
  "fonts.googleapis.com",
  "fonts.gstatic.com",
  "ajax.googleapis.com",
  "static.cloudflareinsights.com",
  "www.googletagmanager.com",
];

const TRACKER_DOMAINS = [
  "doubleclick.net",
  "googlesyndication.com",
  "googleadservices.com",
  "google-analytics.com",
  "www.google-analytics.com",
  "stats.g.doubleclick.net",
  "connect.facebook.net",
  "analytics.twitter.com",
  "bat.bing.com",
  "snap.licdn.com",
  "px.ads.linkedin.com",
];

const FIRST_PARTY_ASSET_PATHS = [
  "/assets/app.js",
  "/assets/vendor.js",
  "/assets/app.css",
  "/assets/logo.png",
  "/assets/hero.jpg",
  "/api/v1/graphql",
  "/api/v1/data",
  "/api/v2/search",
  "/static/chunk.js",
];

const THIRD_PARTY_PATHS = [
  "/gtm.js?id=GTM-" + "XXXX",
  "/analytics.js",
  "/collect",
  "/g/collect",
  "/pixel.gif",
  "/beacon",
  "/tag/js/gpt.js",
  "/pagead/js/adsbygoogle.js",
  "/api/v1/pixel",
];

const REQUEST_TYPE_DIST: { item: RequestType; w: number }[] = [
  { item: "script", w: 22 },
  { item: "image", w: 28 },
  { item: "stylesheet", w: 10 },
  { item: "xmlhttprequest", w: 16 },
  { item: "font", w: 6 },
  { item: "ping", w: 8 },
  { item: "media", w: 6 },
  { item: "websocket", w: 2 },
  { item: "other", w: 2 },
];

function makeQueryParams(
  r: () => number,
  kind: "tracker" | "asset" | "worst",
): string {
  if (kind === "asset") {
    const v = randHex(r, 8);
    return `?v=${v}`;
  }

  if (kind === "tracker") {
    const cid = randHex(r, 16);
    const tid = "UA-" + randInt(r, 100000, 999999) + "-1";
    const gclid = randAlnum(r, 24);
    return `?cid=${cid}&tid=${tid}&gclid=${gclid}&utm_source=${pick(["google", "twitter", "newsletter"], r)}&utm_campaign=${randAlnum(r, 10)}`;
  }

  // worst
  const parts: string[] = [];
  const n = 60;
  for (let i = 0; i < n; i++) {
    parts.push(`${randAlnum(r, 6)}=${randAlnum(r, randInt(r, 8, 32))}`);
  }
  parts.push(`gclid=${randAlnum(r, 32)}`);
  parts.push(`fbclid=${randAlnum(r, 32)}`);
  parts.push(`msclkid=${randAlnum(r, 32)}`);
  return `?${parts.join("&")}`;
}

function makeLongPath(r: () => number): string {
  const segs = randInt(r, 8, 18);
  const parts: string[] = [];
  for (let i = 0; i < segs; i++) parts.push(randAlnum(r, randInt(r, 6, 24)));
  return `/${parts.join("/")}`;
}

function generateSyntheticWorkload(
  pages: number,
  reqsPerPage: number,
  seed: number,
): BenchRequest[] {
  const r = createRng(seed);
  const out: BenchRequest[] = [];

  let reqCounter = 0;
  let tabId = 1;

  for (let p = 0; p < pages; p++) {
    const site = pick(TOP_SITES, r);
    const siteOrigin = `https://www.${site}/`;

    // main frame
    out.push({
      url: siteOrigin,
      type: "main_frame",
      initiator: undefined,
      tabId,
      frameId: 0,
      requestId: `t${tabId}:r${reqCounter++}`,
    });

    // sometimes embed a subframe (ads or video)
    const hasSubframe = r() < 0.25;
    let subframeId = 0;
    if (hasSubframe) {
      subframeId = randInt(r, 1, 3);
      const frameHost =
        r() < 0.6 ? pick(TRACKER_DOMAINS, r) : pick(CDN_DOMAINS, r);
      out.push({
        url: `https://${frameHost}/frame.html${makeQueryParams(r, "tracker")}`,
        type: "sub_frame",
        initiator: siteOrigin,
        tabId,
        frameId: subframeId,
        requestId: `t${tabId}:r${reqCounter++}`,
      });
    }

    for (let i = 0; i < reqsPerPage; i++) {
      const type = weightedPick(REQUEST_TYPE_DIST, r);

      // third party probability depends on type
      const thirdPartyChance =
        type === "ping"
          ? 0.85
          : type === "font"
            ? 0.75
            : type === "script"
              ? 0.45
              : type === "xmlhttprequest"
                ? 0.35
                : type === "image"
                  ? 0.55
                  : 0.3;

      const isThirdParty = r() < thirdPartyChance;

      const host = isThirdParty
        ? r() < 0.6
          ? pick(TRACKER_DOMAINS, r)
          : pick(CDN_DOMAINS, r)
        : r() < 0.3
          ? `static.${site}`
          : `www.${site}`;

      const basePath = isThirdParty
        ? pick(THIRD_PARTY_PATHS, r)
        : pick(FIRST_PARTY_ASSET_PATHS, r);

      // inject some "worst-case" URL shapes
      const worst = r() < 0.03;
      const path = worst ? makeLongPath(r) : basePath;

      const qp = worst
        ? makeQueryParams(r, "worst")
        : isThirdParty
          ? makeQueryParams(r, "tracker")
          : r() < 0.6
            ? makeQueryParams(r, "asset")
            : "";

      const initiator = type === "main_frame" ? undefined : siteOrigin;
      const frameId = hasSubframe && r() < 0.2 ? subframeId : 0;

      out.push({
        url: `https://${host}${path}${qp}`,
        type,
        initiator,
        tabId,
        frameId,
        requestId: `t${tabId}:r${reqCounter++}`,
      });
    }

    tabId++;
    if (tabId > 8) tabId = 1;
  }

  return out;
}

// ----------------------- bench core -----------------------

function formatResult(r: BenchResult): string {
  return [
    `${r.name}:`,
    `  Ops: ${r.opCount.toLocaleString()}`,
    `  Total: ${r.totalMs.toFixed(2)} ms`,
    `  Avg: ${r.avgUs.toFixed(2)} us`,
    `  P50: ${r.p50Us.toFixed(2)} us`,
    `  P95: ${r.p95Us.toFixed(2)} us`,
    `  P99: ${r.p99Us.toFixed(2)} us`,
    `  Throughput: ${r.opsPerSec.toLocaleString()} ops/sec`,
    `  Blocked: ${r.blockedPct.toFixed(1)}%`,
  ].join("\n");
}

function runBenchBatched(
  name: string,
  requests: BenchRequest[],
  iterations: number,
  sampleBatchOps: number,
  fn: (req: BenchRequest) => number,
): BenchResult {
  const samplesUs: number[] = [];
  let blocked = 0;

  const totalOps = requests.length * iterations;

  let ops = 0;
  let batchOps = 0;
  let batchStart = nowNs();
  const start = nowNs();

  let sink = 0;

  for (let it = 0; it < iterations; it++) {
    for (let i = 0; i < requests.length; i++) {
      const v = fn(requests[i]!);
      sink ^= v;

      if (v !== 0) blocked++;

      ops++;
      batchOps++;

      if (batchOps === sampleBatchOps) {
        const dt = nowNs() - batchStart;
        const usPerOp = Number(dt) / 1000 / sampleBatchOps;
        samplesUs.push(usPerOp);
        batchOps = 0;
        batchStart = nowNs();
      }
    }
  }

  // Prevent DCE
  if (sink === 0x7fffffff) console.log("sink", sink);

  const end = nowNs();
  const totalMs = Number(end - start) / 1e6;

  samplesUs.sort((a, b) => a - b);

  const avgUs = (totalMs * 1000) / totalOps;

  const p50Us = percentile(samplesUs, 0.5);
  const p95Us = percentile(samplesUs, 0.95);
  const p99Us = percentile(samplesUs, 0.99);

  return {
    name,
    opCount: totalOps,
    totalMs,
    avgUs,
    p50Us,
    p95Us,
    p99Us,
    opsPerSec: Math.round(totalOps / (totalMs / 1000)),
    blockedPct: (blocked / totalOps) * 100,
  };
}

function warmup(
  requests: BenchRequest[],
  warmupOps: number,
  fn: (req: BenchRequest) => void,
): void {
  if (requests.length === 0) return;
  const loops = Math.max(1, Math.floor(warmupOps / requests.length));
  for (let i = 0; i < loops; i++) {
    for (const req of requests) fn(req);
  }
}

// ----------------------- main -----------------------

async function main(): Promise<void> {
  const opt = parseArgs(process.argv);

  console.log("=".repeat(72));
  console.log("BetterBlocker Realistic Benchmark");
  console.log("=".repeat(72));
  console.log(`Input: ${opt.inputPath}`);
  console.log(`Snapshot: ${opt.snapshotPath}`);
  console.log(`Compile: ${opt.compile ? "yes" : "no"}`);
  console.log(`Mode: ${opt.mode}`);
  console.log(`Iterations: ${opt.iterations.toLocaleString()}`);
  console.log(`Warmup ops: ${opt.warmupOps.toLocaleString()}`);
  console.log(`Sample batch ops: ${opt.sampleBatchOps.toLocaleString()}`);
  console.log();

  if (opt.compile) {
    await compileSnapshot(opt.inputPath, opt.snapshotPath);
    console.log();
  }

  console.log("Loading WASM module...");
  const wasm = await loadWasm();

  console.log("Loading snapshot...");
  const snapshot = await loadSnapshot(opt.snapshotPath);
  wasm.init(snapshot);

  const info = wasm.get_snapshot_info();
  console.log(`Snapshot size: ${(info.size / 1024).toFixed(1)} KB`);
  console.log();

  let requests: BenchRequest[] = [];
  if (opt.tracePath) {
    console.log(`Loading trace: ${opt.tracePath} (limit ${opt.traceLimit})`);
    requests = await loadTraceJsonl(opt.tracePath, opt.traceLimit);
  } else {
    console.log(
      `Generating synthetic workload: pages=${opt.syntheticPages}, reqs/page=${opt.syntheticReqsPerPage}, seed=${opt.seed}`,
    );
    requests = generateSyntheticWorkload(
      opt.syntheticPages,
      opt.syntheticReqsPerPage,
      opt.seed,
    );
  }

  console.log(`Dataset size: ${requests.length.toLocaleString()} requests`);
  console.log();

  // Warmup should match the API you benchmark.
  console.log("Warming up...");
  if (opt.mode === "should_block" || opt.mode === "both") {
    warmup(requests, opt.warmupOps, (req) => {
      wasm.should_block(req.url, req.type, req.initiator);
    });
  }
  if (opt.mode === "match_request" || opt.mode === "both") {
    warmup(requests, opt.warmupOps, (req) => {
      wasm.match_request(
        req.url,
        req.type,
        req.initiator,
        req.tabId,
        req.frameId,
        req.requestId,
      );
    });
  }
  console.log("Warmup done.");
  console.log();

  // Baseline overhead bench (JS loop only). Useful sanity check.
  const baseline = runBenchBatched(
    "Baseline (JS loop only)",
    requests,
    Math.max(1, Math.floor(opt.iterations / 4)),
    opt.sampleBatchOps,
    (req) => req.url.length & 1,
  );
  console.log(formatResult(baseline));
  console.log();

  if (opt.mode === "should_block" || opt.mode === "both") {
    const res = runBenchBatched(
      "should_block (core matcher)",
      requests,
      opt.iterations,
      opt.sampleBatchOps,
      (req) => (wasm.should_block(req.url, req.type, req.initiator) ? 1 : 0),
    );
    console.log(formatResult(res));
    console.log();
  }

  if (opt.mode === "match_request" || opt.mode === "both") {
    const res = runBenchBatched(
      "match_request (extension-facing API)",
      requests,
      opt.iterations,
      opt.sampleBatchOps,
      (req) => {
        const out = wasm.match_request(
          req.url,
          req.type,
          req.initiator,
          req.tabId,
          req.frameId,
          req.requestId,
        );
        // Touch a field so the object is actually used.
        return out.decision ? 1 : 0;
      },
    );
    console.log(formatResult(res));
    console.log();
  }

  console.log("Notes:");
  console.log(
    "- p50/p95/p99 are computed from per-batch wall-time samples divided by batch size.",
  );
  console.log(
    "- For the most realistic numbers, feed a real trace via --trace (jsonl).",
  );
  console.log(
    "- If match_request looks much slower than should_block, JS object creation is a major cost.",
  );
}

main().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
