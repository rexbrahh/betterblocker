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
