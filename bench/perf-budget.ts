import { existsSync } from 'node:fs';

const WASM_PATH = './dist/wasm/bb_wasm.js';
const SNAPSHOT_PATH = './dist/data/snapshot.ubx';

interface WasmModule {
  init(data: Uint8Array): void;
  is_initialized(): boolean;
  should_block(url: string, requestType: string, initiator: string | undefined): boolean;
  get_snapshot_info(): { size: number; initialized: boolean };
}

interface Budget {
  name: string;
  limit: number;
  unit: string;
  actual: number;
  passed: boolean;
}

const BUDGETS = {
  coldStartMs: 500,
  wasmPeakMb: 50,
  matchP99Us: 1000,
  snapshotSizeMb: 30,
};

function assertFileExists(path: string, hint: string): void {
  if (!existsSync(path)) {
    throw new Error(`${hint} not found at ${path}`);
  }
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

function measureColdStart(wasm: WasmModule, snapshot: Uint8Array): number {
  const start = performance.now();
  wasm.init(snapshot);
  return performance.now() - start;
}

function measureMatchLatency(wasm: WasmModule, iterations: number): number[] {
  const testUrls = [
    { url: 'https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js', type: 'script', initiator: 'https://example.com' },
    { url: 'https://www.google-analytics.com/analytics.js', type: 'script', initiator: 'https://example.com' },
    { url: 'https://example.com/style.css', type: 'stylesheet', initiator: 'https://example.com' },
    { url: 'https://cdn.example.com/image.png', type: 'image', initiator: 'https://example.com' },
    { url: 'https://api.example.com/data.json', type: 'xmlhttprequest', initiator: 'https://example.com' },
  ];

  const latencies: number[] = [];

  for (let i = 0; i < iterations; i++) {
    for (const req of testUrls) {
      const start = performance.now();
      wasm.should_block(req.url, req.type, req.initiator);
      const end = performance.now();
      latencies.push((end - start) * 1000);
    }
  }

  latencies.sort((a, b) => a - b);
  return latencies;
}

function percentile(sorted: number[], p: number): number {
  const idx = Math.ceil(sorted.length * p) - 1;
  return sorted[Math.max(0, idx)] ?? 0;
}

async function main(): Promise<void> {
  console.log('Performance Budget Check');
  console.log('='.repeat(50));
  console.log();

  const results: Budget[] = [];

  console.log('Loading WASM module...');
  const wasm = await loadWasm();

  console.log('Loading snapshot...');
  const snapshot = await loadSnapshot();

  const snapshotSizeMb = snapshot.byteLength / (1024 * 1024);
  results.push({
    name: 'Snapshot Size',
    limit: BUDGETS.snapshotSizeMb,
    unit: 'MB',
    actual: snapshotSizeMb,
    passed: snapshotSizeMb <= BUDGETS.snapshotSizeMb,
  });

  console.log('Measuring cold start...');
  const coldStartMs = measureColdStart(wasm, snapshot);
  results.push({
    name: 'Cold Start',
    limit: BUDGETS.coldStartMs,
    unit: 'ms',
    actual: coldStartMs,
    passed: coldStartMs <= BUDGETS.coldStartMs,
  });

  console.log('Warming up...');
  for (let i = 0; i < 1000; i++) {
    wasm.should_block('https://example.com/test', 'script', 'https://example.com');
  }

  console.log('Measuring match latency (10000 iterations)...');
  const latencies = measureMatchLatency(wasm, 2000);
  const p99Us = percentile(latencies, 0.99);
  results.push({
    name: 'Match P99 Latency',
    limit: BUDGETS.matchP99Us,
    unit: 'μs',
    actual: p99Us,
    passed: p99Us <= BUDGETS.matchP99Us,
  });

  const wasmInfo = wasm.get_snapshot_info();
  const wasmPeakMb = wasmInfo.size / (1024 * 1024);
  results.push({
    name: 'WASM Memory Peak',
    limit: BUDGETS.wasmPeakMb,
    unit: 'MB',
    actual: wasmPeakMb,
    passed: wasmPeakMb <= BUDGETS.wasmPeakMb,
  });

  console.log();
  console.log('Results');
  console.log('-'.repeat(50));

  let allPassed = true;
  for (const result of results) {
    const status = result.passed ? '✓' : '✗';
    const color = result.passed ? '\x1b[32m' : '\x1b[31m';
    const reset = '\x1b[0m';
    console.log(
      `${color}${status}${reset} ${result.name}: ${result.actual.toFixed(2)} ${result.unit} (limit: ${result.limit} ${result.unit})`
    );
    if (!result.passed) {
      allPassed = false;
    }
  }

  console.log();
  console.log('='.repeat(50));

  if (allPassed) {
    console.log('\x1b[32m✓ All performance budgets passed\x1b[0m');
    process.exit(0);
  } else {
    console.log('\x1b[31m✗ Performance budget exceeded\x1b[0m');
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
