import { $ } from 'bun';
import { existsSync } from 'node:fs';
import { mkdir } from 'node:fs/promises';
import { generateTestRequests, generateRealisticMix, type TestRequest } from './test-urls';

const WASM_PATH = './dist/wasm/bb_wasm.js';
const DIST_DATA_DIR = './dist/data';
const SNAPSHOT_PATH = `${DIST_DATA_DIR}/snapshot.ubx`;
const FILTER_LIST_PATH = './testdata/test-filters.txt';

interface WasmModule {
  init(data: Uint8Array): void;
  is_initialized(): boolean;
  match_request(
    url: string,
    requestType: string,
    initiator: string | undefined,
    tabId: number,
    frameId: number,
    requestId: string
  ): { decision: number; ruleId: number; listId: number; redirectUrl?: string };
  should_block(url: string, requestType: string, initiator: string | undefined): boolean;
  get_snapshot_info(): { size: number; initialized: boolean };
}

interface BenchResult {
  name: string;
  iterations: number;
  totalMs: number;
  avgUs: number;
  p50Us: number;
  p95Us: number;
  p99Us: number;
  opsPerSec: number;
}

function assertFileExists(path: string, hint: string): void {
  if (!existsSync(path)) {
    throw new Error(`${hint} not found at ${path}`);
  }
}

async function compileSnapshot(): Promise<void> {
  assertFileExists(FILTER_LIST_PATH, 'Filter list');
  await mkdir(DIST_DATA_DIR, { recursive: true });
  
  console.log('Compiling filter list to snapshot...');
  await $`cargo run --release --package bb-cli -- compile -i ${FILTER_LIST_PATH} -o ${SNAPSHOT_PATH} -v`.quiet();
  console.log('Snapshot compiled.');
}

async function loadWasm(): Promise<WasmModule> {
  const wasmJsPath = Bun.resolveSync(WASM_PATH, process.cwd());
  const wasmBinaryPath = wasmJsPath.replace('.js', '_bg.wasm');

  assertFileExists(wasmJsPath, 'WASM JS module');
  assertFileExists(wasmBinaryPath, 'WASM binary');
  
  const jsModule = await import(wasmJsPath);
  const wasmBytes = await Bun.file(wasmBinaryPath).arrayBuffer();
  
  await jsModule.default({ module_or_path: wasmBytes });
  
  return jsModule as unknown as WasmModule;
}

async function loadSnapshot(): Promise<Uint8Array> {
  assertFileExists(SNAPSHOT_PATH, 'Snapshot file');
  const file = Bun.file(SNAPSHOT_PATH);
  const buffer = await file.arrayBuffer();
  return new Uint8Array(buffer);
}

function percentile(sorted: number[], p: number): number {
  const idx = Math.ceil(sorted.length * p) - 1;
  return sorted[Math.max(0, idx)] ?? 0;
}

function runBenchmark(
  name: string,
  requests: TestRequest[],
  wasm: WasmModule,
  iterations: number
): BenchResult {
  const latencies: number[] = [];
  
  for (let iter = 0; iter < iterations; iter++) {
    for (const req of requests) {
      const start = performance.now();
      wasm.should_block(req.url, req.type, req.initiator);
      const end = performance.now();
      latencies.push((end - start) * 1000);
    }
  }
  
  latencies.sort((a, b) => a - b);
  
  const totalMs = latencies.reduce((a, b) => a + b, 0) / 1000;
  const avgUs = latencies.reduce((a, b) => a + b, 0) / latencies.length;
  
  return {
    name,
    iterations: latencies.length,
    totalMs,
    avgUs,
    p50Us: percentile(latencies, 0.50),
    p95Us: percentile(latencies, 0.95),
    p99Us: percentile(latencies, 0.99),
    opsPerSec: Math.round(latencies.length / (totalMs / 1000)),
  };
}

function formatResult(result: BenchResult): string {
  return [
    `${result.name}:`,
    `  Iterations: ${result.iterations.toLocaleString()}`,
    `  Total time: ${result.totalMs.toFixed(2)}ms`,
    `  Avg latency: ${result.avgUs.toFixed(2)}μs`,
    `  P50 latency: ${result.p50Us.toFixed(2)}μs`,
    `  P95 latency: ${result.p95Us.toFixed(2)}μs`,
    `  P99 latency: ${result.p99Us.toFixed(2)}μs`,
    `  Throughput:  ${result.opsPerSec.toLocaleString()} ops/sec`,
  ].join('\n');
}

async function warmup(wasm: WasmModule, requests: TestRequest[]): Promise<void> {
  for (let i = 0; i < 100; i++) {
    for (const req of requests) {
      wasm.should_block(req.url, req.type, req.initiator);
    }
  }
}

async function main(): Promise<void> {
  console.log('='.repeat(60));
  console.log('BetterBlocker WASM Benchmark');
  console.log('='.repeat(60));
  console.log();
  
  await compileSnapshot();
  console.log();
  
  console.log('Loading WASM module...');
  const wasm = await loadWasm();
  
  console.log('Loading snapshot...');
  const snapshot = await loadSnapshot();
  wasm.init(snapshot);
  
  const info = wasm.get_snapshot_info();
  console.log(`Snapshot size: ${(info.size / 1024).toFixed(1)} KB`);
  console.log();
  
  const realisticMix = generateRealisticMix();
  const randomRequests = generateTestRequests(1000);
  
  console.log('Warming up...');
  await warmup(wasm, realisticMix);
  console.log();
  
  console.log('-'.repeat(60));
  console.log('Benchmark: Realistic Mix (10 requests, 10000 iterations)');
  console.log('-'.repeat(60));
  const realisticResult = runBenchmark('Realistic Mix', realisticMix, wasm, 10000);
  console.log(formatResult(realisticResult));
  console.log();
  
  console.log('-'.repeat(60));
  console.log('Benchmark: Random Requests (1000 requests, 100 iterations)');
  console.log('-'.repeat(60));
  const randomResult = runBenchmark('Random Requests', randomRequests, wasm, 100);
  console.log(formatResult(randomResult));
  console.log();
  
  console.log('-'.repeat(60));
  console.log('Benchmark: Single Hot Path (1 request, 100000 iterations)');
  console.log('-'.repeat(60));
  const hotPathResult = runBenchmark('Hot Path', [realisticMix[0]!], wasm, 100000);
  console.log(formatResult(hotPathResult));
  console.log();
  
  console.log('='.repeat(60));
  console.log('Summary');
  console.log('='.repeat(60));
  console.log(`Target: <5ms per request (5000μs)`);
  console.log(`Achieved: ${realisticResult.p99Us.toFixed(2)}μs P99`);
  console.log(`Status: ${realisticResult.p99Us < 5000 ? '✓ PASS' : '✗ FAIL'}`);
}

main().catch(console.error);
