/// <reference types="chrome"/>

import { MatchDecision } from '../shared/types.js';

declare const browser: typeof chrome | undefined;

const api = (typeof browser !== 'undefined' ? browser : chrome) as typeof chrome;

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
}

let wasm: WasmExports | null = null;
let blockCount = 0;
let enabled = true;
let initPromise: Promise<void> | null = null;

async function loadWasm(): Promise<WasmExports> {
  const wasmUrl = api.runtime.getURL('wasm/bb_wasm_bg.wasm');
  const jsUrl = api.runtime.getURL('wasm/bb_wasm.js');

  const jsModule = await import(jsUrl);
  const wasmResponse = await fetch(wasmUrl);
  const wasmBytes = await wasmResponse.arrayBuffer();

  await jsModule.default(wasmBytes);

  return jsModule as unknown as WasmExports;
}

async function loadSnapshot(): Promise<Uint8Array> {
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
          });
          return true;

        case 'toggleEnabled':
          enabled = !enabled;
          if (enabled) {
             api.browserAction.setIcon({ path: "icons/icon48.png" });
          }
          sendResponse({ enabled });
          return true;

        case 'reloadSnapshot':
          loadSnapshot()
            .then((data) => {
              if (data.length > 0 && wasm) {
                wasm.init(data);
                sendResponse({ success: true });
              } else {
                sendResponse({ success: false, error: 'No snapshot data' });
              }
            })
            .catch((e: Error) => {
              sendResponse({ success: false, error: e.message });
            });
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
