type TraceEntry = {
  url: string;
  type: string;
  initiator?: string;
  tabId: number;
  frameId: number;
  requestId: string;
};

let enabled = false;
let maxEntries = 50_000;
let entries: TraceEntry[] = [];

export function traceIsEnabled(): boolean {
  return enabled;
}

export function traceConfigure(on: boolean, max: number = 50_000): void {
  enabled = on;
  maxEntries = Math.max(1_000, Math.min(500_000, Math.floor(max)));
  if (!enabled) {
    entries = [];
  }
}

export function traceMaybeRecord(details: any): void {
  if (!enabled) return;
  if (entries.length >= maxEntries) return;

  const url = String(details?.url || '');
  if (!url) return;

  const initiator =
    (typeof details?.initiator === 'string' && details.initiator) ||
    (typeof details?.documentUrl === 'string' && details.documentUrl) ||
    (typeof details?.originUrl === 'string' && details.originUrl) ||
    undefined;

  entries.push({
    url,
    type: String(details?.type || 'other'),
    initiator,
    tabId: Number.isFinite(details?.tabId) ? details.tabId : -1,
    frameId: Number.isFinite(details?.frameId) ? details.frameId : 0,
    requestId: String(details?.requestId || ''),
  });
}

export function traceExportJsonl(): string {
  return entries.map((entry) => JSON.stringify(entry)).join('\n') + '\n';
}

export function traceStats(): { enabled: boolean; count: number; max: number } {
  return { enabled, count: entries.length, max: maxEntries };
}

type PerfPhase = 'beforeRequest' | 'headersReceived';

type PerfBucket = {
  count: number;
  min: number;
  max: number;
  p50: number;
  p95: number;
  p99: number;
};

let perfEnabled = false;
let perfMaxEntries = 100_000;
const perfEntries: Record<PerfPhase, number[]> = {
  beforeRequest: [],
  headersReceived: [],
};

function computePercentile(values: number[], percentile: number): number {
  if (values.length === 0) {
    return 0;
  }
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.max(0, Math.floor(sorted.length * percentile)));
  return sorted[idx] ?? 0;
}

function summarize(values: number[]): PerfBucket {
  if (values.length === 0) {
    return { count: 0, min: 0, max: 0, p50: 0, p95: 0, p99: 0 };
  }
  const sorted = [...values].sort((a, b) => a - b);
  return {
    count: sorted.length,
    min: sorted[0] ?? 0,
    max: sorted[sorted.length - 1] ?? 0,
    p50: computePercentile(sorted, 0.5),
    p95: computePercentile(sorted, 0.95),
    p99: computePercentile(sorted, 0.99),
  };
}

export function perfConfigure(on: boolean, max: number = 100_000): void {
  perfEnabled = on;
  perfMaxEntries = Math.max(1_000, Math.min(1_000_000, Math.floor(max)));
  if (!perfEnabled) {
    perfEntries.beforeRequest = [];
    perfEntries.headersReceived = [];
  }
}

export function perfMaybeRecord(phase: PerfPhase, durationMs: number): void {
  if (!perfEnabled) {
    return;
  }
  const bucket = perfEntries[phase];
  if (!bucket || bucket.length >= perfMaxEntries) {
    return;
  }
  bucket.push(durationMs);
}

export function perfStats(): { enabled: boolean; beforeRequest: PerfBucket; headersReceived: PerfBucket } {
  return {
    enabled: perfEnabled,
    beforeRequest: summarize(perfEntries.beforeRequest),
    headersReceived: summarize(perfEntries.headersReceived),
  };
}

export function perfExportJson(): string {
  return JSON.stringify(perfEntries);
}
