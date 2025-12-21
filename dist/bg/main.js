(() => {
  // src/shared/types.ts
  var DOCUMENT_TYPES = 64 /* MAIN_FRAME */ | 32 /* SUBDOCUMENT */;
  var ALL_PARTIES = 1 /* FIRST_PARTY */ | 2 /* THIRD_PARTY */;

  // src/bg/snapshot-store.ts
  var DB_NAME = "betterblocker";
  var DB_VERSION = 1;
  var STORE_NAME = "snapshots";
  var ACTIVE_KEY = "active";
  function openDb() {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(DB_NAME, DB_VERSION);
      request.onupgradeneeded = () => {
        const db = request.result;
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          db.createObjectStore(STORE_NAME, { keyPath: "key" });
        }
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error ?? new Error("Failed to open IndexedDB"));
    });
  }
  async function loadStoredSnapshot() {
    const db = await openDb();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readonly");
      const store = tx.objectStore(STORE_NAME);
      const request = store.get(ACTIVE_KEY);
      request.onsuccess = () => {
        resolve(request.result ?? null);
      };
      request.onerror = () => {
        reject(request.error ?? new Error("Failed to read snapshot"));
      };
      tx.oncomplete = () => db.close();
      tx.onerror = () => {
        db.close();
      };
    });
  }
  async function saveStoredSnapshot(record) {
    const db = await openDb();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      const store = tx.objectStore(STORE_NAME);
      store.put({ key: ACTIVE_KEY, ...record });
      tx.oncomplete = () => {
        db.close();
        resolve();
      };
      tx.onerror = () => {
        db.close();
        reject(tx.error ?? new Error("Failed to save snapshot"));
      };
    });
  }
  async function clearStoredSnapshot() {
    const db = await openDb();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      const store = tx.objectStore(STORE_NAME);
      store.delete(ACTIVE_KEY);
      tx.oncomplete = () => {
        db.close();
        resolve();
      };
      tx.onerror = () => {
        db.close();
        reject(tx.error ?? new Error("Failed to clear snapshot"));
      };
    });
  }

  // src/bg/trace.ts
  var enabled = false;
  var maxEntries = 50000;
  var entries = [];
  function traceConfigure(on, max = 50000) {
    enabled = on;
    maxEntries = Math.max(1000, Math.min(500000, Math.floor(max)));
    if (!enabled) {
      entries = [];
    }
  }
  function traceMaybeRecord(details) {
    if (!enabled)
      return;
    if (entries.length >= maxEntries)
      return;
    const url = String(details?.url || "");
    if (!url)
      return;
    const initiator = typeof details?.initiator === "string" && details.initiator || typeof details?.documentUrl === "string" && details.documentUrl || typeof details?.originUrl === "string" && details.originUrl || undefined;
    entries.push({
      url,
      type: String(details?.type || "other"),
      initiator,
      tabId: Number.isFinite(details?.tabId) ? details.tabId : -1,
      frameId: Number.isFinite(details?.frameId) ? details.frameId : 0,
      requestId: String(details?.requestId || "")
    });
  }
  function traceExportJsonl() {
    return entries.map((entry) => JSON.stringify(entry)).join(`
`) + `
`;
  }
  function traceStats() {
    return { enabled, count: entries.length, max: maxEntries };
  }

  // src/bg/main.ts
  var api = typeof browser !== "undefined" ? browser : chrome;
  var STORAGE_KEY = "filterLists";
  var wasm = null;
  var blockCount = 0;
  var enabled2 = true;
  var snapshotStats = null;
  var initPromise = null;
  async function loadWasm(cacheBust) {
    const cacheSuffix = cacheBust ? `?v=${cacheBust}` : "";
    const wasmUrl = `${api.runtime.getURL("wasm/bb_wasm_bg.wasm")}${cacheSuffix}`;
    const jsUrl = `${api.runtime.getURL("wasm/bb_wasm.js")}${cacheSuffix}`;
    const jsModule = await import(jsUrl);
    const wasmResponse = await fetch(wasmUrl);
    const wasmBytes = await wasmResponse.arrayBuffer();
    await jsModule.default({ module_or_path: wasmBytes });
    return jsModule;
  }
  async function loadSnapshot() {
    try {
      const stored = await loadStoredSnapshot();
      if (stored && stored.data.byteLength > 0) {
        snapshotStats = stored.stats;
        return stored.data;
      }
    } catch (e) {
      console.warn("[BetterBlocker] Failed to load stored snapshot:", e);
    }
    snapshotStats = null;
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
  async function swapMatcher(snapshot) {
    const cacheBust = Date.now().toString(36);
    const nextWasm = await loadWasm(cacheBust);
    if (snapshot && snapshot.length > 0) {
      nextWasm.init(snapshot);
    }
    wasm = nextWasm;
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
  async function getLists() {
    return new Promise((resolve) => {
      api.storage.sync.get([STORAGE_KEY], (result) => {
        const lists = result[STORAGE_KEY];
        resolve(lists ?? []);
      });
    });
  }
  async function saveLists(lists) {
    return new Promise((resolve) => {
      api.storage.sync.set({ [STORAGE_KEY]: lists }, () => resolve());
    });
  }
  async function fetchListText(url) {
    const response = await fetch(url, { cache: "no-store" });
    if (!response.ok) {
      throw new Error(`Failed to fetch list: ${response.status}`);
    }
    return response.text();
  }
  async function compileAndStoreLists() {
    if (initPromise) {
      await initPromise;
    }
    if (!wasm) {
      throw new Error("WASM module not loaded");
    }
    const lists = await getLists();
    const enabledLists = lists.filter((list) => list.enabled && list.url.trim().length > 0);
    if (enabledLists.length === 0) {
      await clearStoredSnapshot();
      snapshotStats = null;
      await saveLists(lists.map((list) => ({
        ...list,
        ruleCount: 0,
        lastUpdated: list.lastUpdated
      })));
      return { stats: null, snapshot: null };
    }
    const listTexts = await Promise.all(enabledLists.map((list) => fetchListText(list.url)));
    const compileResult = wasm.compile_filter_lists(listTexts);
    const now = new Date().toISOString();
    const listStats = compileResult.listStats ?? [];
    const updatedLists = lists.map((list) => {
      const idx = enabledLists.findIndex((enabled3) => enabled3.id === list.id);
      if (idx === -1) {
        return { ...list, ruleCount: 0 };
      }
      const stats = listStats[idx];
      return {
        ...list,
        ruleCount: stats ? stats.rulesAfter : 0,
        lastUpdated: now
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
        rulesAfter: stat.rulesAfter
      }))
    };
    await saveStoredSnapshot({
      data: snapshotBytes,
      stats: snapshotStats,
      updatedAt: now,
      sourceUrls: enabledLists.map((list) => list.url)
    });
    return { stats: snapshotStats, snapshot: snapshotBytes };
  }
  function onBeforeRequest(details) {
    traceMaybeRecord(details);
    if (!enabled2 || !wasm?.is_initialized()) {
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
            enabled: enabled2,
            initialized: wasm?.is_initialized() ?? false,
            snapshotInfo: wasm?.get_snapshot_info() ?? null,
            snapshotStats
          });
          return true;
        case "toggleEnabled":
          enabled2 = !enabled2;
          if (enabled2) {
            api.browserAction.setIcon({ path: "icons/icon48.png" });
          }
          sendResponse({ enabled: enabled2 });
          return true;
        case "updateList":
        case "updateAllLists":
        case "listsChanged":
          compileAndStoreLists().then(({ stats, snapshot }) => swapMatcher(snapshot).then(() => {
            sendResponse({ success: true, snapshotStats: stats });
          })).catch((e) => {
            console.error("[BetterBlocker] List update failed:", e);
            sendResponse({ success: false, error: e.message });
          });
          return true;
        case "reloadSnapshot":
          loadSnapshot().then((snapshot) => swapMatcher(snapshot.length > 0 ? snapshot : null)).then(() => {
            sendResponse({ success: true });
          }).catch((e) => {
            sendResponse({ success: false, error: e.message });
          });
          return true;
        case "trace.start":
          traceConfigure(true, message.maxEntries ?? 50000);
          sendResponse({ ok: true, stats: traceStats() });
          return true;
        case "trace.stop":
          traceConfigure(false);
          sendResponse({ ok: true, stats: traceStats() });
          return true;
        case "trace.stats":
          sendResponse({ ok: true, stats: traceStats() });
          return true;
        case "trace.export":
          sendResponse({ ok: true, jsonl: traceExportJsonl(), stats: traceStats() });
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
