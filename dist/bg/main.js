(() => {
  // src/shared/types.ts
  var DOCUMENT_TYPES = 64 /* MAIN_FRAME */ | 32 /* SUBDOCUMENT */;
  var ALL_PARTIES = 1 /* FIRST_PARTY */ | 2 /* THIRD_PARTY */;

  // src/bg/main.ts
  var api = typeof browser !== "undefined" ? browser : chrome;
  var wasm = null;
  var blockCount = 0;
  var enabled = true;
  var initPromise = null;
  async function loadWasm() {
    const wasmUrl = api.runtime.getURL("wasm/bb_wasm_bg.wasm");
    const jsUrl = api.runtime.getURL("wasm/bb_wasm.js");
    const jsModule = await import(jsUrl);
    const wasmResponse = await fetch(wasmUrl);
    const wasmBytes = await wasmResponse.arrayBuffer();
    await jsModule.default({ module_or_path: wasmBytes });
    return jsModule;
  }
  async function loadSnapshot() {
    const snapshotUrl = api.runtime.getURL("data/snapshot.ubx");
    try {
      const response = await fetch(snapshotUrl);
      if (!response.ok) {
        throw new Error(`Failed to load snapshot: ${response.status}`);
      }
      const buffer = await response.arrayBuffer();
      return new Uint8Array(buffer);
    } catch {
      console.warn("[BetterBlocker] No bundled snapshot found, starting with empty ruleset");
      return new Uint8Array(0);
    }
  }
  async function initialize() {
    try {
      console.log("[BetterBlocker] Initializing...");
      wasm = await loadWasm();
      console.log("[BetterBlocker] WASM module loaded");
      const snapshot = await loadSnapshot();
      if (snapshot.length > 0) {
        wasm.init(snapshot);
        const info = wasm.get_snapshot_info();
        console.log(`[BetterBlocker] Snapshot loaded: ${info.size} bytes`);
      } else {
        console.log("[BetterBlocker] No snapshot loaded, blocking disabled");
      }
      console.log("[BetterBlocker] Ready");
    } catch (e) {
      console.error("[BetterBlocker] Initialization failed:", e);
      throw e;
    }
  }
  function onBeforeRequest(details) {
    if (!enabled || !wasm?.is_initialized()) {
      return;
    }
    if (details.tabId < 0) {
      return;
    }
    const initiator = details.initiator ?? details.originUrl ?? details.documentUrl;
    try {
      const result = wasm.match_request(details.url, details.type, initiator, details.tabId, details.frameId, details.requestId);
      switch (result.decision) {
        case 1 /* BLOCK */:
          blockCount++;
          return { cancel: true };
        case 2 /* REDIRECT */:
          if (result.redirectUrl) {
            blockCount++;
            const redirectUrl = result.redirectUrl.startsWith("/") ? api.runtime.getURL(`resources${result.redirectUrl}`) : result.redirectUrl;
            return { redirectUrl };
          }
          return { cancel: true };
        case 3 /* REMOVEPARAM */:
          if (result.redirectUrl) {
            return { redirectUrl: result.redirectUrl };
          }
          return;
        default:
          return;
      }
    } catch (e) {
      console.error("[BetterBlocker] Match error:", e);
      return;
    }
  }
  function setupWebRequest() {
    const filter = {
      urls: ["http://*/*", "https://*/*", "ws://*/*", "wss://*/*"]
    };
    api.webRequest.onBeforeRequest.addListener(onBeforeRequest, filter, ["blocking"]);
    console.log("[BetterBlocker] webRequest listener registered");
  }
  function setupMessageHandlers() {
    api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
      switch (message.type) {
        case "getStats":
          sendResponse({
            blockCount,
            enabled,
            initialized: wasm?.is_initialized() ?? false,
            snapshotInfo: wasm?.get_snapshot_info() ?? null
          });
          return true;
        case "toggleEnabled":
          enabled = !enabled;
          if (enabled) {
            api.browserAction.setIcon({ path: "icons/icon48.png" });
          }
          sendResponse({ enabled });
          return true;
        case "reloadSnapshot":
          loadSnapshot().then((data) => {
            if (data.length > 0 && wasm) {
              wasm.init(data);
              sendResponse({ success: true });
            } else {
              sendResponse({ success: false, error: "No snapshot data" });
            }
          }).catch((e) => {
            sendResponse({ success: false, error: e.message });
          });
          return true;
        default:
          return false;
      }
    });
  }
  initPromise = initialize();
  setupWebRequest();
  setupMessageHandlers();
  initPromise.catch((e) => {
    console.error("[BetterBlocker] Failed to initialize:", e);
  });
})();
