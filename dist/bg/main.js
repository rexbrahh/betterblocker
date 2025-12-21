(() => {
  // src/shared/types.ts
  var DOCUMENT_TYPES = 64 /* MAIN_FRAME */ | 32 /* SUBDOCUMENT */;
  var ALL_PARTIES = 1 /* FIRST_PARTY */ | 2 /* THIRD_PARTY */;
  var DEFAULT_SETTINGS = {
    enabled: true,
    cosmeticsEnabled: true,
    scriptletsEnabled: true,
    dynamicFilteringEnabled: true,
    removeparamEnabled: true,
    cspEnabled: true,
    responseHeaderEnabled: true,
    disabledSites: []
  };

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
  var perfEnabled = false;
  var perfMaxEntries = 1e5;
  var perfEntries = {
    beforeRequest: [],
    headersReceived: []
  };
  function computePercentile(values, percentile) {
    if (values.length === 0) {
      return 0;
    }
    const sorted = [...values].sort((a, b) => a - b);
    const idx = Math.min(sorted.length - 1, Math.max(0, Math.floor(sorted.length * percentile)));
    return sorted[idx] ?? 0;
  }
  function summarize(values) {
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
      p99: computePercentile(sorted, 0.99)
    };
  }
  function perfConfigure(on, max = 1e5) {
    perfEnabled = on;
    perfMaxEntries = Math.max(1000, Math.min(1e6, Math.floor(max)));
    if (!perfEnabled) {
      perfEntries.beforeRequest = [];
      perfEntries.headersReceived = [];
    }
  }
  function perfMaybeRecord(phase, durationMs) {
    if (!perfEnabled) {
      return;
    }
    const bucket = perfEntries[phase];
    if (!bucket || bucket.length >= perfMaxEntries) {
      return;
    }
    bucket.push(durationMs);
  }
  function perfStats() {
    return {
      enabled: perfEnabled,
      beforeRequest: summarize(perfEntries.beforeRequest),
      headersReceived: summarize(perfEntries.headersReceived)
    };
  }
  function perfExportJson() {
    return JSON.stringify(perfEntries);
  }

  // src/bg/main.ts
  var api = typeof browser !== "undefined" ? browser : chrome;
  var STORAGE_KEY = "filterLists";
  var DYNAMIC_RULES_KEY = "dynamicRules";
  var SETTINGS_KEY = "settings";
  var DEFAULT_LISTS = [
    {
      id: "oisd-big-bundled",
      name: "OISD Big (bundled)",
      url: api.runtime.getURL("data/oisd_big_abp.txt"),
      enabled: true,
      ruleCount: 0,
      lastUpdated: null,
      pinned: true,
      version: "202512210305",
      homepage: "https://oisd.nl",
      license: "https://github.com/sjhgvr/oisd/blob/main/LICENSE",
      source: "bundled"
    },
    {
      id: "hagezi-ultimate-bundled",
      name: "HaGeZi's Ultimate (bundled)",
      url: api.runtime.getURL("data/ultimate.txt"),
      enabled: true,
      ruleCount: 0,
      lastUpdated: null,
      pinned: true,
      version: "2025.1220.1821.33",
      homepage: "https://github.com/hagezi/dns-blocklists",
      license: "https://github.com/hagezi/dns-blocklists/blob/main/LICENSE",
      source: "bundled"
    }
  ];
  var UPDATE_ALARM_NAME = "listUpdate";
  var UPDATE_INTERVAL_MINUTES = 24 * 60;
  var LIST_FETCH_TIMEOUT_MS = 30000;
  var LIST_MAX_BYTES = 25 * 1024 * 1024;
  var topFrameByTab = new Map;
  var blockedByTab = new Map;
  var mainFrameRequestIdByTab = new Map;
  var dynamicRules = [];
  var settings = { ...DEFAULT_SETTINGS };
  var removeparamRedirects = new Map;
  var REMOVEPARAM_TTL_MS = 1e4;
  var BADGE_COLOR = "#d94848";
  function clearRemoveparamHistory(tabId) {
    for (const key of removeparamRedirects.keys()) {
      if (key.startsWith(`${tabId}:`)) {
        removeparamRedirects.delete(key);
      }
    }
  }
  function pruneRemoveparamHistory(now) {
    for (const [key, entry] of removeparamRedirects) {
      if (now - entry.ts >= REMOVEPARAM_TTL_MS) {
        removeparamRedirects.delete(key);
      }
    }
  }
  function getRemoveparamKey(details) {
    return `${details.tabId}:${details.frameId}:${details.url}`;
  }
  function shouldSkipRemoveparam(details, redirectUrl) {
    const now = Date.now();
    pruneRemoveparamHistory(now);
    const key = getRemoveparamKey(details);
    const existing = removeparamRedirects.get(key);
    if (existing && now - existing.ts < REMOVEPARAM_TTL_MS) {
      return true;
    }
    removeparamRedirects.set(key, { url: redirectUrl, ts: now });
    return false;
  }
  async function loadDynamicRules() {
    return new Promise((resolve) => {
      api.storage.local.get([DYNAMIC_RULES_KEY], (result) => {
        const rules = result[DYNAMIC_RULES_KEY];
        dynamicRules = Array.isArray(rules) ? rules : [];
        resolve();
      });
    });
  }
  async function saveDynamicRules(rules) {
    return new Promise((resolve) => {
      api.storage.local.set({ [DYNAMIC_RULES_KEY]: rules }, () => resolve());
    });
  }
  function normalizeSettings(value) {
    const raw = value ?? {};
    const merged = { ...DEFAULT_SETTINGS, ...raw };
    const disabledSites = Array.isArray(merged.disabledSites) ? merged.disabledSites.filter((site) => typeof site === "string" && site.trim().length > 0) : [];
    return {
      enabled: merged.enabled !== false,
      cosmeticsEnabled: merged.cosmeticsEnabled !== false,
      scriptletsEnabled: merged.scriptletsEnabled !== false,
      dynamicFilteringEnabled: merged.dynamicFilteringEnabled !== false,
      removeparamEnabled: merged.removeparamEnabled !== false,
      cspEnabled: merged.cspEnabled !== false,
      responseHeaderEnabled: merged.responseHeaderEnabled !== false,
      disabledSites
    };
  }
  async function loadSettings() {
    return new Promise((resolve) => {
      api.storage.sync.get([SETTINGS_KEY], (result) => {
        const stored = result[SETTINGS_KEY];
        settings = normalizeSettings(stored);
        resolve();
      });
    });
  }
  async function saveSettings(next) {
    return new Promise((resolve) => {
      api.storage.sync.set({ [SETTINGS_KEY]: next }, () => resolve());
    });
  }
  function applySettings(update) {
    settings = normalizeSettings({ ...settings, ...update });
    return settings;
  }
  function getSitePattern(url) {
    const host = extractHost(url);
    if (!host) {
      return null;
    }
    const etld1 = getEtld1(host);
    return etld1 || host;
  }
  function isSiteDisabled(url) {
    if (!url) {
      return false;
    }
    const host = extractHost(url);
    if (!host) {
      return false;
    }
    return settings.disabledSites.some((pattern) => hostMatches(pattern, host));
  }
  function updateBadge(tabId) {
    if (tabId < 0) {
      return;
    }
    const siteUrl = topFrameByTab.get(tabId);
    if (!settings.enabled || isSiteDisabled(siteUrl)) {
      api.browserAction.setBadgeText({ tabId, text: "" });
      return;
    }
    const count = blockedByTab.get(tabId) ?? 0;
    const text = count > 0 ? String(count) : "";
    api.browserAction.setBadgeText({ tabId, text });
    if (text) {
      api.browserAction.setBadgeBackgroundColor({ tabId, color: BADGE_COLOR });
    }
  }
  function updateAllBadges() {
    for (const tabId of blockedByTab.keys()) {
      updateBadge(tabId);
    }
  }
  function resetTabBlockCount(tabId, requestId) {
    if (tabId < 0) {
      return;
    }
    const prevRequestId = mainFrameRequestIdByTab.get(tabId);
    if (prevRequestId === requestId) {
      return;
    }
    mainFrameRequestIdByTab.set(tabId, requestId);
    blockedByTab.set(tabId, 0);
    updateBadge(tabId);
  }
  function incrementTabBlockCount(tabId) {
    if (tabId < 0) {
      return;
    }
    blockCount += 1;
    blockedByTab.set(tabId, (blockedByTab.get(tabId) ?? 0) + 1);
    updateBadge(tabId);
  }
  function getTabBlockCount(tabId) {
    if (tabId < 0) {
      return 0;
    }
    return blockedByTab.get(tabId) ?? 0;
  }
  function extractHost(url) {
    try {
      return new URL(url).hostname;
    } catch {
      return "";
    }
  }
  function getEtld1(host) {
    if (!host)
      return "";
    if (wasm?.get_etld1_js) {
      return wasm.get_etld1_js(host);
    }
    return host;
  }
  function hostMatches(pattern, host) {
    if (!pattern || pattern === "*") {
      return true;
    }
    if (!host) {
      return false;
    }
    if (host === pattern) {
      return true;
    }
    return host.endsWith(`.${pattern}`);
  }
  function targetMatches(pattern, reqHost, reqEtld1, isThirdParty) {
    if (!pattern || pattern === "*") {
      return true;
    }
    if (pattern === "3p" || pattern === "third-party") {
      return isThirdParty;
    }
    if (pattern === "1p" || pattern === "first-party") {
      return !isThirdParty;
    }
    if (reqEtld1 && reqEtld1 === pattern) {
      return true;
    }
    return hostMatches(pattern, reqHost);
  }
  function typeMatches(ruleType, requestType) {
    if (!ruleType || ruleType === "*") {
      return true;
    }
    const normalized = ruleType.toLowerCase();
    if (normalized === "document") {
      return requestType === "main_frame" || requestType === "sub_frame";
    }
    if (normalized === "subdocument" || normalized === "sub_frame") {
      return requestType === "sub_frame";
    }
    if (normalized === "main_frame") {
      return requestType === "main_frame";
    }
    if (normalized === "xhr") {
      return requestType === "xmlhttprequest";
    }
    return normalized === requestType;
  }
  function matchDynamicRules(details, initiator) {
    if (!settings.dynamicFilteringEnabled || dynamicRules.length === 0) {
      return 0 /* NOOP */;
    }
    const reqHost = extractHost(details.url);
    const siteUrl = initiator ?? details.url;
    const siteHost = extractHost(siteUrl);
    const siteEtld1 = getEtld1(siteHost);
    const reqEtld1 = getEtld1(reqHost);
    const isThirdParty = siteEtld1.length > 0 && reqEtld1.length > 0 && siteEtld1 !== reqEtld1;
    let bestAction = 0 /* NOOP */;
    let bestScore = -1;
    let bestIndex = -1;
    for (let i = 0;i < dynamicRules.length; i++) {
      const rule = dynamicRules[i];
      if (!rule) {
        continue;
      }
      const sitePattern = rule.site?.toLowerCase() ?? "*";
      const targetPattern = rule.target?.toLowerCase() ?? "*";
      const typePattern = rule.type?.toLowerCase() ?? "*";
      if (!hostMatches(sitePattern, siteHost)) {
        continue;
      }
      if (!targetMatches(targetPattern, reqHost, reqEtld1, isThirdParty)) {
        continue;
      }
      if (!typeMatches(typePattern, details.type)) {
        continue;
      }
      let score = 0;
      if (sitePattern !== "*")
        score += 1;
      if (targetPattern !== "*")
        score += 1;
      if (typePattern !== "*")
        score += 1;
      if (score > bestScore || score === bestScore && i > bestIndex) {
        bestScore = score;
        bestIndex = i;
        bestAction = rule.action;
      }
    }
    return bestAction;
  }
  function normalizeContextUrl(value) {
    if (!value) {
      return;
    }
    if (value === "null" || value === "about:blank") {
      return;
    }
    return value;
  }
  function updateTopFrame(details) {
    if (details.type === "main_frame" && details.tabId >= 0) {
      topFrameByTab.set(details.tabId, details.url);
      resetTabBlockCount(details.tabId, details.requestId);
    }
  }
  function getContextUrl(details) {
    if (details.type === "main_frame") {
      return details.url;
    }
    const topFrame = topFrameByTab.get(details.tabId);
    if (topFrame) {
      return topFrame;
    }
    return normalizeContextUrl(details.initiator) ?? normalizeContextUrl(details.originUrl) ?? normalizeContextUrl(details.documentUrl);
  }
  var wasm = null;
  var blockCount = 0;
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
  async function loadBundledSnapshot() {
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
    return loadBundledSnapshot();
  }
  async function swapMatcher(snapshot) {
    const cacheBust = Date.now().toString(36);
    const nextWasm = await loadWasm(cacheBust);
    if (snapshot && snapshot.length > 0) {
      try {
        nextWasm.init(snapshot);
        nextWasm.get_snapshot_info();
      } catch (e) {
        console.warn("[BetterBlocker] Snapshot validation failed during swap:", e);
        return false;
      }
    }
    wasm = nextWasm;
    return true;
  }
  async function initialize() {
    try {
      console.log("[BetterBlocker] Initializing...");
      wasm = await loadWasm();
      console.log("[BetterBlocker] WASM module loaded");
      const snapshot = await loadSnapshot();
      if (snapshot.length > 0) {
        try {
          wasm.init(snapshot);
          const info = wasm.get_snapshot_info();
          console.log(`[BetterBlocker] Snapshot loaded: ${info.size} bytes`);
        } catch (e) {
          console.warn("[BetterBlocker] Snapshot failed validation, clearing stored snapshot");
          await clearStoredSnapshot();
          snapshotStats = null;
          const fallback = await loadBundledSnapshot();
          if (fallback.length > 0) {
            try {
              wasm.init(fallback);
              const info = wasm.get_snapshot_info();
              console.log(`[BetterBlocker] Bundled snapshot loaded: ${info.size} bytes`);
            } catch (err) {
              console.warn("[BetterBlocker] Bundled snapshot invalid, blocking disabled");
            }
          } else {
            console.log("[BetterBlocker] No snapshot loaded, blocking disabled");
          }
        }
      } else {
        console.log("[BetterBlocker] No snapshot loaded, blocking disabled");
      }
      await loadDynamicRules();
      await loadSettings();
      const seeded = await ensureDefaultLists();
      if (seeded) {
        setTimeout(() => {
          compileAndStoreLists().catch((e) => {
            console.error("[BetterBlocker] Default list compile failed:", e);
          });
        }, 0);
      }
      setupUpdateSchedule();
      updateAllBadges();
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
  async function ensureDefaultLists() {
    const lists = await getLists();
    if (lists.length > 0) {
      return false;
    }
    await saveLists(DEFAULT_LISTS);
    return true;
  }
  function setupUpdateSchedule() {
    if (!api.alarms) {
      return;
    }
    api.alarms.create(UPDATE_ALARM_NAME, { periodInMinutes: UPDATE_INTERVAL_MINUTES });
    api.alarms.onAlarm.addListener((alarm) => {
      if (alarm.name !== UPDATE_ALARM_NAME) {
        return;
      }
      compileAndStoreLists().catch((e) => {
        console.error("[BetterBlocker] Scheduled list update failed:", e);
      });
    });
  }
  async function readResponseTextWithLimit(response, controller, maxBytes) {
    if (!response.body) {
      const buffer = await response.arrayBuffer();
      if (buffer.byteLength > maxBytes) {
        throw new Error(`List exceeds max size of ${maxBytes} bytes`);
      }
      return new TextDecoder("utf-8").decode(buffer);
    }
    const reader = response.body.getReader();
    const chunks = [];
    let total = 0;
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      if (!value) {
        continue;
      }
      total += value.byteLength;
      if (total > maxBytes) {
        controller.abort();
        throw new Error(`List exceeds max size of ${maxBytes} bytes`);
      }
      chunks.push(value);
    }
    const data = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      data.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return new TextDecoder("utf-8").decode(data);
  }
  async function fetchListText(url) {
    const controller = new AbortController;
    const timeoutId = setTimeout(() => controller.abort(), LIST_FETCH_TIMEOUT_MS);
    try {
      const response = await fetch(url, { cache: "no-store", signal: controller.signal });
      if (!response.ok) {
        throw new Error(`Failed to fetch list: ${response.status}`);
      }
      const contentLength = response.headers.get("content-length");
      if (contentLength) {
        const length = Number(contentLength);
        if (Number.isFinite(length) && length > LIST_MAX_BYTES) {
          throw new Error(`List exceeds max size of ${LIST_MAX_BYTES} bytes`);
        }
      }
      return await readResponseTextWithLimit(response, controller, LIST_MAX_BYTES);
    } catch (err) {
      if (controller.signal.aborted) {
        throw new Error(`List fetch timed out after ${LIST_FETCH_TIMEOUT_MS}ms`);
      }
      throw err;
    } finally {
      clearTimeout(timeoutId);
    }
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
      await swapMatcher(null);
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
      const idx = enabledLists.findIndex((enabled2) => enabled2.id === list.id);
      if (idx === -1) {
        return { ...list, ruleCount: 0 };
      }
      const stats2 = listStats[idx];
      return {
        ...list,
        ruleCount: stats2 ? stats2.rulesAfter : 0,
        lastUpdated: now
      };
    });
    const snapshotBytes = compileResult.snapshot;
    const swapped = await swapMatcher(snapshotBytes);
    if (!swapped) {
      throw new Error("Snapshot validation failed during swap");
    }
    const stats = {
      rulesBefore: compileResult.rulesBefore,
      rulesAfter: compileResult.rulesAfter,
      listStats: listStats.map((stat) => ({
        lines: stat.lines,
        rulesBefore: stat.rulesBefore,
        rulesAfter: stat.rulesAfter
      }))
    };
    if (typeof compileResult.rulesDeduped === "number") {
      stats.rulesDeduped = compileResult.rulesDeduped;
    }
    if (typeof compileResult.badfilterRules === "number") {
      stats.badfilterRules = compileResult.badfilterRules;
    }
    if (typeof compileResult.badfilteredRules === "number") {
      stats.badfilteredRules = compileResult.badfilteredRules;
    }
    snapshotStats = stats;
    await saveLists(updatedLists);
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
    const perfStart = performance.now();
    const finalize = (response) => {
      perfMaybeRecord("beforeRequest", performance.now() - perfStart);
      return response;
    };
    if (details.tabId < 0) {
      return finalize(undefined);
    }
    updateTopFrame(details);
    if (!settings.enabled || !wasm?.is_initialized()) {
      return finalize(undefined);
    }
    const initiator = getContextUrl(details);
    if (isSiteDisabled(initiator ?? details.url)) {
      return finalize(undefined);
    }
    const dynamicDecision = matchDynamicRules(details, initiator);
    if (dynamicDecision === 1 /* BLOCK */) {
      incrementTabBlockCount(details.tabId);
      return finalize({ cancel: true });
    }
    if (dynamicDecision === 2 /* ALLOW */) {
      return finalize(undefined);
    }
    try {
      const result = wasm.match_request(details.url, details.type, initiator, details.tabId, details.frameId, details.requestId);
      switch (result.decision) {
        case 1 /* BLOCK */:
          incrementTabBlockCount(details.tabId);
          return finalize({ cancel: true });
        case 2 /* REDIRECT */:
          if (result.redirectUrl) {
            incrementTabBlockCount(details.tabId);
            const redirectUrl = result.redirectUrl.startsWith("/") ? api.runtime.getURL(`resources${result.redirectUrl}`) : result.redirectUrl;
            return finalize({ redirectUrl });
          }
          incrementTabBlockCount(details.tabId);
          return finalize({ cancel: true });
        case 3 /* REMOVEPARAM */:
          if (!settings.removeparamEnabled) {
            return finalize(undefined);
          }
          if (result.redirectUrl) {
            if (shouldSkipRemoveparam(details, result.redirectUrl)) {
              return finalize(undefined);
            }
            return finalize({ redirectUrl: result.redirectUrl });
          }
          return finalize(undefined);
        default:
          return finalize(undefined);
      }
    } catch (e) {
      console.error("[BetterBlocker] Match error:", e);
      return finalize(undefined);
    }
  }
  function onHeadersReceived(details) {
    const perfStart = performance.now();
    const finalize = (response) => {
      perfMaybeRecord("headersReceived", performance.now() - perfStart);
      return response;
    };
    if (details.tabId < 0) {
      return finalize(undefined);
    }
    if (!settings.enabled || !wasm?.is_initialized()) {
      return finalize(undefined);
    }
    const headers = details.responseHeaders;
    if (!headers || headers.length === 0) {
      return finalize(undefined);
    }
    const initiator = getContextUrl(details);
    if (isSiteDisabled(initiator ?? details.url)) {
      return finalize(undefined);
    }
    try {
      const result = wasm.match_response_headers(details.url, details.type, initiator, details.tabId, details.frameId, details.requestId, headers);
      if (result.cancel) {
        incrementTabBlockCount(details.tabId);
        return finalize({ cancel: true });
      }
      const removeHeaders = settings.responseHeaderEnabled ? result.removeHeaders ?? [] : [];
      let responseHeaders = headers;
      if (removeHeaders.length > 0) {
        const removeSet = new Set(removeHeaders.map((name) => name.toLowerCase()));
        responseHeaders = headers.filter((header) => !removeSet.has(header.name.toLowerCase()));
      }
      if (settings.cspEnabled && result.csp && result.csp.length > 0) {
        responseHeaders = [...responseHeaders];
        for (const value of result.csp) {
          responseHeaders.push({ name: "Content-Security-Policy", value });
        }
        return finalize({ responseHeaders });
      }
      if (removeHeaders.length > 0) {
        return finalize({ responseHeaders });
      }
      return finalize(undefined);
    } catch (e) {
      console.error("[BetterBlocker] Header match error:", e);
      return finalize(undefined);
    }
  }
  function setupWebRequest() {
    const filter = {
      urls: ["http://*/*", "https://*/*", "ws://*/*", "wss://*/*"]
    };
    api.webRequest.onBeforeRequest.addListener(onBeforeRequest, filter, ["blocking"]);
    api.webRequest.onHeadersReceived.addListener(onHeadersReceived, filter, ["blocking", "responseHeaders"]);
    console.log("[BetterBlocker] webRequest listener registered");
  }
  function setupTabTracking() {
    api.tabs.onRemoved.addListener((tabId) => {
      topFrameByTab.delete(tabId);
      blockedByTab.delete(tabId);
      mainFrameRequestIdByTab.delete(tabId);
      clearRemoveparamHistory(tabId);
    });
  }
  function setupMessageHandlers() {
    api.runtime.onMessage.addListener((message, sender, sendResponse) => {
      switch (message.type) {
        case "getStats": {
          const tabId = typeof message.tabId === "number" ? message.tabId : sender.tab?.id ?? -1;
          const siteUrl = typeof message.url === "string" ? message.url : sender.tab?.url ?? topFrameByTab.get(tabId);
          sendResponse({
            blockCount,
            enabled: settings.enabled,
            initialized: wasm?.is_initialized() ?? false,
            snapshotInfo: wasm?.get_snapshot_info() ?? null,
            snapshotStats,
            tabBlockCount: getTabBlockCount(tabId),
            siteDisabled: isSiteDisabled(siteUrl)
          });
          return true;
        }
        case "dynamic.get":
          sendResponse({ rules: dynamicRules });
          return true;
        case "dynamic.set": {
          const rules = Array.isArray(message.rules) ? message.rules : [];
          dynamicRules = rules;
          saveDynamicRules(rules).then(() => sendResponse({ ok: true })).catch((e) => sendResponse({ ok: false, error: e.message }));
          return true;
        }
        case "cosmetic.get": {
          const url = typeof message.url === "string" ? message.url : sender.url ?? sender.tab?.url;
          if (!url || !wasm?.is_initialized() || !settings.enabled || isSiteDisabled(url) || !settings.cosmeticsEnabled && !settings.scriptletsEnabled) {
            sendResponse({ css: "", enableGeneric: true, procedural: [], scriptlets: [] });
            return true;
          }
          const tabId = sender.tab?.id ?? -1;
          const frameId = sender.frameId ?? 0;
          const requestId = message.requestId ?? "cosmetic";
          const result = wasm.match_cosmetics(url, "main_frame", undefined, tabId, frameId, requestId);
          if (!settings.cosmeticsEnabled) {
            result.css = "";
            result.enableGeneric = false;
            result.procedural = [];
          }
          if (!settings.scriptletsEnabled || frameId !== 0) {
            result.scriptlets = [];
          }
          sendResponse(result);
          return true;
        }
        case "settings.get":
          sendResponse({ settings });
          return true;
        case "settings.update": {
          const update = message.settings ?? {};
          const next = applySettings(update);
          saveSettings(next).then(() => {
            updateAllBadges();
            sendResponse({ ok: true, settings: next });
          }).catch((e) => sendResponse({ ok: false, error: e.message, settings }));
          return true;
        }
        case "site.toggle": {
          const url = typeof message.url === "string" ? message.url : sender.tab?.url ?? sender.url;
          const enabledValue = typeof message.enabled === "boolean" ? message.enabled : undefined;
          if (!url || typeof enabledValue !== "boolean") {
            sendResponse({ ok: false, error: "Missing url or enabled flag" });
            return true;
          }
          const pattern = getSitePattern(url);
          if (!pattern) {
            sendResponse({ ok: false, error: "Invalid site url" });
            return true;
          }
          const disabledSites = new Set(settings.disabledSites);
          if (enabledValue) {
            disabledSites.delete(pattern);
          } else {
            disabledSites.add(pattern);
          }
          const next = applySettings({ disabledSites: Array.from(disabledSites) });
          const tabId = sender.tab?.id ?? message.tabId ?? -1;
          saveSettings(next).then(() => {
            if (tabId >= 0) {
              updateBadge(tabId);
            }
            sendResponse({ ok: true, disabled: !enabledValue, sitePattern: pattern, settings: next });
          }).catch((e) => sendResponse({ ok: false, error: e.message, settings }));
          return true;
        }
        case "toggleEnabled": {
          const next = applySettings({ enabled: !settings.enabled });
          saveSettings(next).then(() => {
            updateAllBadges();
            if (next.enabled) {
              api.browserAction.setIcon({ path: "icons/icon48.png" });
            }
            sendResponse({ enabled: next.enabled });
          }).catch((e) => sendResponse({ enabled: settings.enabled, error: e.message }));
          return true;
        }
        case "updateList":
        case "updateAllLists":
        case "listsChanged":
          compileAndStoreLists().then(({ stats }) => {
            sendResponse({ success: true, snapshotStats: stats });
          }).catch((e) => {
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
        case "perf.start":
          perfConfigure(true, message.maxEntries ?? 1e5);
          sendResponse({ ok: true, stats: perfStats() });
          return true;
        case "perf.stop":
          perfConfigure(false);
          sendResponse({ ok: true, stats: perfStats() });
          return true;
        case "perf.stats":
          sendResponse({ ok: true, stats: perfStats() });
          return true;
        case "perf.export":
          sendResponse({ ok: true, json: perfExportJson(), stats: perfStats() });
          return true;
        default:
          return false;
      }
    });
  }
  initPromise = initialize();
  setupWebRequest();
  setupTabTracking();
  setupMessageHandlers();
  initPromise.catch((e) => {
    console.error("[BetterBlocker] Failed to initialize:", e);
  });
})();
