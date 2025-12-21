/// <reference types="chrome"/>

import { MatchDecision } from '../shared/types.js';
import {
  clearStoredSnapshot,
  loadStoredSnapshot,
  saveStoredSnapshot,
  type SnapshotStats,
} from './snapshot-store.js';
import { traceConfigure, traceExportJsonl, traceMaybeRecord, traceStats } from './trace.js';

declare const browser: typeof chrome | undefined;

const api = (typeof browser !== 'undefined' ? browser : chrome) as typeof chrome;

const STORAGE_KEY = 'filterLists';

interface FilterList {
  id: string;
  name: string;
  url: string;
  enabled: boolean;
  ruleCount: number;
  lastUpdated: string | null;
}

interface WasmExports {
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
  compile_filter_lists(list_texts: string[]): {
    snapshot: Uint8Array;
    rulesBefore: number;
    rulesAfter: number;
    listStats: { lines: number; rulesBefore: number; rulesAfter: number }[];
  };
}

let wasm: WasmExports | null = null;
let blockCount = 0;
let enabled = true;
let snapshotStats: SnapshotStats | null = null;
let initPromise: Promise<void> | null = null;

async function loadWasm(cacheBust?: string): Promise<WasmExports> {
  const cacheSuffix = cacheBust ? `?v=${cacheBust}` : '';
  const wasmUrl = `${api.runtime.getURL('wasm/bb_wasm_bg.wasm')}${cacheSuffix}`;
  const jsUrl = `${api.runtime.getURL('wasm/bb_wasm.js')}${cacheSuffix}`;

  const jsModule = await import(jsUrl);
  const wasmResponse = await fetch(wasmUrl);
  const wasmBytes = await wasmResponse.arrayBuffer();

  await jsModule.default({ module_or_path: wasmBytes });

  return jsModule as unknown as WasmExports;
}

async function loadSnapshot(): Promise<Uint8Array> {
  try {
    const stored = await loadStoredSnapshot();
    if (stored && stored.data.byteLength > 0) {
      snapshotStats = stored.stats;
      return stored.data;
    }
  } catch (e) {
    console.warn('[BetterBlocker] Failed to load stored snapshot:', e);
  }

  snapshotStats = null;
  const snapshotUrl = api.runtime.getURL('data/snapshot.ubx');

  try {
    const response = await fetch(snapshotUrl);
    if (!response.ok) {
      throw new Error(`Failed to load snapshot: ${response.status}`);
    }
    const buffer = await response.arrayBuffer();
    return new Uint8Array(buffer);
  } catch {
    console.warn('[BetterBlocker] No bundled snapshot found, starting with empty ruleset');
    return new Uint8Array(0);
  }
}

async function swapMatcher(snapshot: Uint8Array | null): Promise<void> {
  const cacheBust = Date.now().toString(36);
  const nextWasm = await loadWasm(cacheBust);
  if (snapshot && snapshot.length > 0) {
    nextWasm.init(snapshot);
  }
  wasm = nextWasm;
}

async function initialize(): Promise<void> {
  try {
    console.log('[BetterBlocker] Initializing...');

    wasm = await loadWasm();
    console.log('[BetterBlocker] WASM module loaded');

    const snapshot = await loadSnapshot();
    if (snapshot.length > 0) {
      wasm.init(snapshot);
      const info = wasm.get_snapshot_info();
      console.log(`[BetterBlocker] Snapshot loaded: ${info.size} bytes`);
    } else {
      console.log('[BetterBlocker] No snapshot loaded, blocking disabled');
    }

    console.log('[BetterBlocker] Ready');
  } catch (e) {
    console.error('[BetterBlocker] Initialization failed:', e);
    throw e;
  }
}

async function getLists(): Promise<FilterList[]> {
  return new Promise((resolve) => {
    api.storage.sync.get([STORAGE_KEY], (result) => {
      const lists = result[STORAGE_KEY] as FilterList[] | undefined;
      resolve(lists ?? []);
    });
  });
}

async function saveLists(lists: FilterList[]): Promise<void> {
  return new Promise((resolve) => {
    api.storage.sync.set({ [STORAGE_KEY]: lists }, () => resolve());
  });
}

async function fetchListText(url: string): Promise<string> {
  const response = await fetch(url, { cache: 'no-store' });
  if (!response.ok) {
    throw new Error(`Failed to fetch list: ${response.status}`);
  }
  return response.text();
}

async function compileAndStoreLists(): Promise<{ stats: SnapshotStats | null; snapshot: Uint8Array | null }> {
  if (initPromise) {
    await initPromise;
  }

  if (!wasm) {
    throw new Error('WASM module not loaded');
  }

  const lists = await getLists();
  const enabledLists = lists.filter((list) => list.enabled && list.url.trim().length > 0);

  if (enabledLists.length === 0) {
    await clearStoredSnapshot();
    snapshotStats = null;
    await saveLists(
      lists.map((list) => ({
        ...list,
        ruleCount: 0,
        lastUpdated: list.lastUpdated,
      }))
    );
    return { stats: null, snapshot: null };
  }

  const listTexts = await Promise.all(enabledLists.map((list) => fetchListText(list.url)));
  const compileResult = wasm.compile_filter_lists(listTexts);
  const now = new Date().toISOString();

  const listStats = compileResult.listStats ?? [];
  const updatedLists = lists.map((list) => {
    const idx = enabledLists.findIndex((enabled) => enabled.id === list.id);
    if (idx === -1) {
      return { ...list, ruleCount: 0 };
    }
    const stats = listStats[idx];
    return {
      ...list,
      ruleCount: stats ? stats.rulesAfter : 0,
      lastUpdated: now,
    };
  });

  await saveLists(updatedLists);

  const snapshotBytes = compileResult.snapshot;

  snapshotStats = {
    rulesBefore: compileResult.rulesBefore,
    rulesAfter: compileResult.rulesAfter,
    listStats: listStats.map((stat) => ({
      lines: stat.lines,
      rulesBefore: stat.rulesBefore,
      rulesAfter: stat.rulesAfter,
    })),
  };

  await saveStoredSnapshot({
    data: snapshotBytes,
    stats: snapshotStats,
    updatedAt: now,
    sourceUrls: enabledLists.map((list) => list.url),
  });

  return { stats: snapshotStats, snapshot: snapshotBytes };
}

interface RequestDetails {
  requestId: string;
  url: string;
  type: string;
  tabId: number;
  frameId: number;
  initiator?: string;
  originUrl?: string;
  documentUrl?: string;
}

function onBeforeRequest(
  details: RequestDetails
): chrome.webRequest.BlockingResponse | undefined {
  traceMaybeRecord(details);
  if (!enabled || !wasm?.is_initialized()) {
    return undefined;
  }

  if (details.tabId < 0) {
    return undefined;
  }

  const initiator = details.initiator ?? details.originUrl ?? details.documentUrl;

  try {
    const result = wasm.match_request(
      details.url,
      details.type,
      initiator,
      details.tabId,
      details.frameId,
      details.requestId
    );

    switch (result.decision) {
      case MatchDecision.BLOCK:
        blockCount++;
        return { cancel: true };

      case MatchDecision.REDIRECT:
        if (result.redirectUrl) {
          blockCount++;
          const redirectUrl = result.redirectUrl.startsWith('/')
            ? api.runtime.getURL(`resources${result.redirectUrl}`)
            : result.redirectUrl;
          return { redirectUrl };
        }
        return { cancel: true };

      case MatchDecision.REMOVEPARAM:
        if (result.redirectUrl) {
          return { redirectUrl: result.redirectUrl };
        }
        return undefined;

      default:
        return undefined;
    }
  } catch (e) {
    console.error('[BetterBlocker] Match error:', e);
    return undefined;
  }
}

function setupWebRequest(): void {
  const filter: chrome.webRequest.RequestFilter = {
    urls: ['http://*/*', 'https://*/*', 'ws://*/*', 'wss://*/*'],
  };

  api.webRequest.onBeforeRequest.addListener(
    onBeforeRequest as Parameters<typeof api.webRequest.onBeforeRequest.addListener>[0],
    filter,
    ['blocking']
  );

  console.log('[BetterBlocker] webRequest listener registered');
}

interface MessageRequest {
  type: string;
  maxEntries?: number;
}

function setupMessageHandlers(): void {
  api.runtime.onMessage.addListener(
    (
      message: MessageRequest,
      _sender: chrome.runtime.MessageSender,
      sendResponse: (response: unknown) => void
    ) => {
      switch (message.type) {
        case 'getStats':
          sendResponse({
            blockCount,
            enabled,
            initialized: wasm?.is_initialized() ?? false,
            snapshotInfo: wasm?.get_snapshot_info() ?? null,
            snapshotStats,
          });
          return true;

        case 'toggleEnabled':
          enabled = !enabled;
          if (enabled) {
             api.browserAction.setIcon({ path: "icons/icon48.png" });
          }
          sendResponse({ enabled });
          return true;

        case 'updateList':
        case 'updateAllLists':
        case 'listsChanged':
          compileAndStoreLists()
            .then(({ stats, snapshot }) => swapMatcher(snapshot).then(() => {
              sendResponse({ success: true, snapshotStats: stats });
            }))
            .catch((e: Error) => {
              console.error('[BetterBlocker] List update failed:', e);
              sendResponse({ success: false, error: e.message });
            });
          return true;

        case 'reloadSnapshot':
          loadSnapshot()
            .then((snapshot) => swapMatcher(snapshot.length > 0 ? snapshot : null))
            .then(() => {
              sendResponse({ success: true });
            })
            .catch((e: Error) => {
              sendResponse({ success: false, error: e.message });
            });
          return true;

        case 'trace.start':
          traceConfigure(true, message.maxEntries ?? 50_000);
          sendResponse({ ok: true, stats: traceStats() });
          return true;

        case 'trace.stop':
          traceConfigure(false);
          sendResponse({ ok: true, stats: traceStats() });
          return true;

        case 'trace.stats':
          sendResponse({ ok: true, stats: traceStats() });
          return true;

        case 'trace.export':
          sendResponse({ ok: true, jsonl: traceExportJsonl(), stats: traceStats() });
          return true;

        default:
          return false;
      }
    }
  );
}

initPromise = initialize();
setupWebRequest();
setupMessageHandlers();

initPromise.catch((e) => {
  console.error('[BetterBlocker] Failed to initialize:', e);
});
