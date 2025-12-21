/// <reference types="chrome"/>

import {
  MatchDecision,
  DynamicAction,
  type DynamicRule,
  type UserSettings,
  DEFAULT_SETTINGS,
} from '../shared/types.js';
import {
  clearStoredSnapshot,
  loadStoredSnapshot,
  saveStoredSnapshot,
  type SnapshotStats,
} from './snapshot-store.js';
import {
  traceConfigure,
  traceExportJsonl,
  traceMaybeRecord,
  traceStats,
  perfConfigure,
  perfMaybeRecord,
  perfStats,
  perfExportJson,
} from './trace.js';

declare const browser: typeof chrome | undefined;

const api = (typeof browser !== 'undefined' ? browser : chrome) as typeof chrome;

const STORAGE_KEY = 'filterLists';
const DYNAMIC_RULES_KEY = 'dynamicRules';
const SETTINGS_KEY = 'settings';
const DEFAULT_LISTS: FilterList[] = [
  {
    id: 'oisd-big-bundled',
    name: 'OISD Big (bundled)',
    url: api.runtime.getURL('data/oisd_big_abp.txt'),
    enabled: true,
    ruleCount: 0,
    lastUpdated: null,
    pinned: true,
    version: '202512210305',
    homepage: 'https://oisd.nl',
    license: 'https://github.com/sjhgvr/oisd/blob/main/LICENSE',
    source: 'bundled',
  },
  {
    id: 'hagezi-ultimate-bundled',
    name: "HaGeZi's Ultimate (bundled)",
    url: api.runtime.getURL('data/ultimate.txt'),
    enabled: true,
    ruleCount: 0,
    lastUpdated: null,
    pinned: true,
    version: '2025.1220.1821.33',
    homepage: 'https://github.com/hagezi/dns-blocklists',
    license: 'https://github.com/hagezi/dns-blocklists/blob/main/LICENSE',
    source: 'bundled',
  },
];
const UPDATE_ALARM_NAME = 'listUpdate';
const UPDATE_INTERVAL_MINUTES = 24 * 60;
const LIST_FETCH_TIMEOUT_MS = 30_000;
const LIST_MAX_BYTES = 25 * 1024 * 1024;
const topFrameByTab = new Map<number, string>();
const blockedByTab = new Map<number, number>();
const mainFrameRequestIdByTab = new Map<number, string>();
let dynamicRules: DynamicRule[] = [];
let settings: UserSettings = { ...DEFAULT_SETTINGS };
const removeparamRedirects = new Map<string, { url: string; ts: number }>();
const REMOVEPARAM_TTL_MS = 10_000;
const BADGE_COLOR = '#d94848';

function clearRemoveparamHistory(tabId: number): void {
  for (const key of removeparamRedirects.keys()) {
    if (key.startsWith(`${tabId}:`)) {
      removeparamRedirects.delete(key);
    }
  }
}

function pruneRemoveparamHistory(now: number): void {
  for (const [key, entry] of removeparamRedirects) {
    if (now - entry.ts >= REMOVEPARAM_TTL_MS) {
      removeparamRedirects.delete(key);
    }
  }
}

function getRemoveparamKey(details: RequestDetails): string {
  return `${details.tabId}:${details.frameId}:${details.url}`;
}

function shouldSkipRemoveparam(details: RequestDetails, redirectUrl: string): boolean {
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

async function loadDynamicRules(): Promise<void> {
  return new Promise((resolve) => {
    api.storage.local.get([DYNAMIC_RULES_KEY], (result) => {
      const rules = result[DYNAMIC_RULES_KEY] as DynamicRule[] | undefined;
      dynamicRules = Array.isArray(rules) ? rules : [];
      resolve();
    });
  });
}

async function saveDynamicRules(rules: DynamicRule[]): Promise<void> {
  return new Promise((resolve) => {
    api.storage.local.set({ [DYNAMIC_RULES_KEY]: rules }, () => resolve());
  });
}

function normalizeSettings(value?: Partial<UserSettings>): UserSettings {
  const raw = value ?? {};
  const merged = { ...DEFAULT_SETTINGS, ...raw } as UserSettings;
  const disabledSites = Array.isArray(merged.disabledSites)
    ? (merged.disabledSites as string[]).filter(
        (site) => typeof site === 'string' && site.trim().length > 0
      )
    : [];


  return {
    enabled: merged.enabled !== false,
    cosmeticsEnabled: merged.cosmeticsEnabled !== false,
    scriptletsEnabled: merged.scriptletsEnabled !== false,
    dynamicFilteringEnabled: merged.dynamicFilteringEnabled !== false,
    removeparamEnabled: merged.removeparamEnabled !== false,
    cspEnabled: merged.cspEnabled !== false,
    responseHeaderEnabled: merged.responseHeaderEnabled !== false,
    disabledSites,
  };
}

async function loadSettings(): Promise<void> {
  return new Promise((resolve) => {
    api.storage.sync.get([SETTINGS_KEY], (result) => {
      const stored = result[SETTINGS_KEY] as Partial<UserSettings> | undefined;
      settings = normalizeSettings(stored);
      resolve();
    });
  });
}

async function saveSettings(next: UserSettings): Promise<void> {
  return new Promise((resolve) => {
    api.storage.sync.set({ [SETTINGS_KEY]: next }, () => resolve());
  });
}

function applySettings(update: Partial<UserSettings>): UserSettings {
  settings = normalizeSettings({ ...settings, ...update });
  return settings;
}

function getSitePattern(url: string): string | null {
  const host = extractHost(url);
  if (!host) {
    return null;
  }
  const etld1 = getEtld1(host);
  return etld1 || host;
}

function isSiteDisabled(url?: string): boolean {
  if (!url) {
    return false;
  }
  const host = extractHost(url);
  if (!host) {
    return false;
  }
  return settings.disabledSites.some((pattern) => hostMatches(pattern, host));
}

function updateBadge(tabId: number): void {
  if (tabId < 0) {
    return;
  }
  const siteUrl = topFrameByTab.get(tabId);
  if (!settings.enabled || isSiteDisabled(siteUrl)) {
    api.browserAction.setBadgeText({ tabId, text: '' });
    return;
  }

  const count = blockedByTab.get(tabId) ?? 0;
  const text = count > 0 ? String(count) : '';
  api.browserAction.setBadgeText({ tabId, text });
  if (text) {
    api.browserAction.setBadgeBackgroundColor({ tabId, color: BADGE_COLOR });
  }
}

function updateAllBadges(): void {
  for (const tabId of blockedByTab.keys()) {
    updateBadge(tabId);
  }
}

function resetTabBlockCount(tabId: number, requestId: string): void {
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

function incrementTabBlockCount(tabId: number): void {
  if (tabId < 0) {
    return;
  }
  blockCount += 1;
  blockedByTab.set(tabId, (blockedByTab.get(tabId) ?? 0) + 1);
  updateBadge(tabId);
}

function getTabBlockCount(tabId: number): number {
  if (tabId < 0) {
    return 0;
  }
  return blockedByTab.get(tabId) ?? 0;
}

function extractHost(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return '';
  }
}

function getEtld1(host: string): string {
  if (!host) return '';
  if (wasm?.get_etld1_js) {
    return wasm.get_etld1_js(host);
  }
  return host;
}

function hostMatches(pattern: string, host: string): boolean {
  if (!pattern || pattern === '*') {
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

function targetMatches(pattern: string, reqHost: string, reqEtld1: string, isThirdParty: boolean): boolean {
  if (!pattern || pattern === '*') {
    return true;
  }
  if (pattern === '3p' || pattern === 'third-party') {
    return isThirdParty;
  }
  if (pattern === '1p' || pattern === 'first-party') {
    return !isThirdParty;
  }
  if (reqEtld1 && reqEtld1 === pattern) {
    return true;
  }
  return hostMatches(pattern, reqHost);
}

function typeMatches(ruleType: string, requestType: string): boolean {
  if (!ruleType || ruleType === '*') {
    return true;
  }
  const normalized = ruleType.toLowerCase();
  if (normalized === 'document') {
    return requestType === 'main_frame' || requestType === 'sub_frame';
  }
  if (normalized === 'subdocument' || normalized === 'sub_frame') {
    return requestType === 'sub_frame';
  }
  if (normalized === 'main_frame') {
    return requestType === 'main_frame';
  }
  if (normalized === 'xhr') {
    return requestType === 'xmlhttprequest';
  }
  return normalized === requestType;
}

type DynamicMatch = { action: DynamicAction; rule: DynamicRule | undefined };

function isOverlyBroadDynamicRule(rule?: DynamicRule): boolean {
  if (!rule) {
    return false;
  }
  const sitePattern = rule.site?.toLowerCase() ?? '*';
  const targetPattern = rule.target?.toLowerCase() ?? '*';
  const typePattern = rule.type?.toLowerCase() ?? '*';
  const isGlobalSite = sitePattern === '*';
  const isGlobalTarget = targetPattern === '*';
  const isMainFrameType =
    typePattern === '*' || typePattern === 'main_frame' || typePattern === 'document';
  return isGlobalSite && isGlobalTarget && isMainFrameType;
}

function matchDynamicRules(details: RequestDetails, initiator?: string): DynamicMatch {
  if (!settings.dynamicFilteringEnabled || dynamicRules.length === 0) {
    return { action: DynamicAction.NOOP, rule: undefined };
  }

  const reqHost = extractHost(details.url);
  const siteUrl = initiator ?? details.url;
  const siteHost = extractHost(siteUrl);
  const siteEtld1 = getEtld1(siteHost);
  const reqEtld1 = getEtld1(reqHost);
  const isThirdParty = siteEtld1.length > 0 && reqEtld1.length > 0 && siteEtld1 !== reqEtld1;

  let bestAction = DynamicAction.NOOP;
  let bestRule: DynamicRule | undefined;
  let bestScore = -1;
  let bestIndex = -1;

  for (let i = 0; i < dynamicRules.length; i++) {
    const rule = dynamicRules[i];
    if (!rule) {
      continue;
    }
    const sitePattern = rule.site?.toLowerCase() ?? '*';
    const targetPattern = rule.target?.toLowerCase() ?? '*';
    const typePattern = rule.type?.toLowerCase() ?? '*';

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
    if (sitePattern !== '*') score += 1;
    if (targetPattern !== '*') score += 1;
    if (typePattern !== '*') score += 1;

    if (score > bestScore || (score === bestScore && i > bestIndex)) {
      bestScore = score;
      bestIndex = i;
      bestAction = rule.action;
      bestRule = rule;
    }
  }

  return { action: bestAction, rule: bestRule };
}

function normalizeContextUrl(value?: string): string | undefined {
  if (!value) {
    return undefined;
  }
  if (value === 'null' || value === 'about:blank') {
    return undefined;
  }
  return value;
}

function updateTopFrame(details: RequestDetails): void {
  if (details.type === 'main_frame' && details.tabId >= 0) {
    topFrameByTab.set(details.tabId, details.url);
    resetTabBlockCount(details.tabId, details.requestId);
  }
}

function getContextUrl(details: RequestDetails): string | undefined {
  if (details.type === 'main_frame') {
    return details.url;
  }

  const topFrame = topFrameByTab.get(details.tabId);
  if (topFrame) {
    return topFrame;
  }

  return (
    normalizeContextUrl(details.initiator) ??
    normalizeContextUrl(details.originUrl) ??
    normalizeContextUrl(details.documentUrl)
  );
}

interface FilterList {
  id: string;
  name: string;
  url: string;
  enabled: boolean;
  ruleCount: number;
  lastUpdated: string | null;
  pinned?: boolean;
  version?: string;
  homepage?: string;
  license?: string;
  source?: string;
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
  match_response_headers(
    url: string,
    requestType: string,
    initiator: string | undefined,
    tabId: number,
    frameId: number,
    requestId: string,
    headers: chrome.webRequest.HttpHeader[]
  ): { cancel: boolean; ruleId: number; listId: number; csp?: string[]; removeHeaders?: string[] };
  match_cosmetics(
    url: string,
    requestType: string,
    initiator: string | undefined,
    tabId: number,
    frameId: number,
    requestId: string
  ): { css: string; enableGeneric: boolean; procedural: string[]; scriptlets: { name: string; args: string[] }[] };
  should_block(url: string, requestType: string, initiator: string | undefined): boolean;
  get_snapshot_info(): { size: number; initialized: boolean };
  get_etld1_js?(host: string): string;
  compile_filter_lists(list_texts: string[]): {
    snapshot: Uint8Array;
    rulesBefore: number;
    rulesAfter: number;
    rulesDeduped?: number;
    badfilterRules?: number;
    badfilteredRules?: number;
    listStats: { lines: number; rulesBefore: number; rulesAfter: number }[];
  };
}

let wasm: WasmExports | null = null;
let blockCount = 0;
let snapshotStats: SnapshotStats | null = null;
let initPromise: Promise<void> | null = null;
let autoCompileInFlight: Promise<void> | null = null;

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

async function loadBundledSnapshot(): Promise<Uint8Array> {
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
  return loadBundledSnapshot();
}

async function swapMatcher(snapshot: Uint8Array | null): Promise<boolean> {
  const cacheBust = Date.now().toString(36);
  const nextWasm = await loadWasm(cacheBust);

  if (snapshot && snapshot.length > 0) {
    try {
      nextWasm.init(snapshot);
      nextWasm.get_snapshot_info();
    } catch (e) {
      console.warn('[BetterBlocker] Snapshot validation failed during swap:', e);
      return false;
    }
  }

  wasm = nextWasm;
  return true;
}

async function initialize(): Promise<void> {
  try {
    console.log('[BetterBlocker] Initializing...');

    wasm = await loadWasm();
    console.log('[BetterBlocker] WASM module loaded');

    const snapshot = await loadSnapshot();
    if (snapshot.length > 0) {
      try {
        wasm.init(snapshot);
        const info = wasm.get_snapshot_info();
        console.log(`[BetterBlocker] Snapshot loaded: ${info.size} bytes`);
      } catch (e) {
        console.warn('[BetterBlocker] Snapshot failed validation, clearing stored snapshot');
        await clearStoredSnapshot();
        snapshotStats = null;
        const fallback = await loadBundledSnapshot();
        if (fallback.length > 0) {
          try {
            wasm.init(fallback);
            const info = wasm.get_snapshot_info();
            console.log(`[BetterBlocker] Bundled snapshot loaded: ${info.size} bytes`);
          } catch (err) {
            console.warn('[BetterBlocker] Bundled snapshot invalid, blocking disabled');
          }
        } else {
          console.log('[BetterBlocker] No snapshot loaded, blocking disabled');
        }
      }
    } else {
      console.log('[BetterBlocker] No snapshot loaded, blocking disabled');
    }

    await loadDynamicRules();
    await loadSettings();
    const seeded = await ensureDefaultLists();
    if (seeded) {
      setTimeout(() => {
        void maybeAutoCompile('seeded');
      }, 0);
    } else if (!wasm?.is_initialized()) {
      setTimeout(() => {
        void maybeAutoCompile('startup');
      }, 0);
    }
    setupUpdateSchedule();
    updateAllBadges();

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

async function ensureDefaultLists(): Promise<boolean> {
  const lists = await getLists();
  if (lists.length > 0) {
    return false;
  }
  await saveLists(DEFAULT_LISTS);
  return true;
}

async function maybeAutoCompile(reason: string): Promise<void> {
  if (autoCompileInFlight) {
    return autoCompileInFlight;
  }
  if (!wasm || wasm.is_initialized()) {
    return;
  }
  const lists = await getLists();
  const enabledLists = lists.filter((list) => list.enabled && list.url.trim().length > 0);
  if (enabledLists.length === 0) {
    console.warn(`[BetterBlocker] Auto-compile skipped (${reason}): no enabled lists`);
    return;
  }
  console.warn(`[BetterBlocker] Auto-compile triggered (${reason})`);
  autoCompileInFlight = compileAndStoreLists()
    .then(() => {
      console.log('[BetterBlocker] Auto-compile finished');
    })
    .catch((e: Error) => {
      console.error('[BetterBlocker] Auto-compile failed:', e);
    })
    .finally(() => {
      autoCompileInFlight = null;
    });
  return autoCompileInFlight;
}

function setupUpdateSchedule(): void {
  if (!api.alarms) {
    return;
  }
  api.alarms.create(UPDATE_ALARM_NAME, { periodInMinutes: UPDATE_INTERVAL_MINUTES });
  api.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name !== UPDATE_ALARM_NAME) {
      return;
    }
    compileAndStoreLists().catch((e: Error) => {
      console.error('[BetterBlocker] Scheduled list update failed:', e);
    });
  });
}

async function readResponseTextWithLimit(
  response: Response,
  controller: AbortController,
  maxBytes: number
): Promise<string> {
  if (!response.body) {
    const buffer = await response.arrayBuffer();
    if (buffer.byteLength > maxBytes) {
      throw new Error(`List exceeds max size of ${maxBytes} bytes`);
    }
    return new TextDecoder('utf-8').decode(buffer);
  }

  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
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

  return new TextDecoder('utf-8').decode(data);
}

async function fetchListText(url: string): Promise<string> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), LIST_FETCH_TIMEOUT_MS);

  try {
    const response = await fetch(url, { cache: 'no-store', signal: controller.signal });
    if (!response.ok) {
      throw new Error(`Failed to fetch list: ${response.status}`);
    }

    const contentLength = response.headers.get('content-length');
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
    await swapMatcher(null);
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

  const snapshotBytes = compileResult.snapshot;
  const swapped = await swapMatcher(snapshotBytes);
  if (!swapped) {
    throw new Error('Snapshot validation failed during swap');
  }

  const stats: SnapshotStats = {
    rulesBefore: compileResult.rulesBefore,
    rulesAfter: compileResult.rulesAfter,
    listStats: listStats.map((stat) => ({
      lines: stat.lines,
      rulesBefore: stat.rulesBefore,
      rulesAfter: stat.rulesAfter,
    })),
  };

  if (typeof compileResult.rulesDeduped === 'number') {
    stats.rulesDeduped = compileResult.rulesDeduped;
  }
  if (typeof compileResult.badfilterRules === 'number') {
    stats.badfilterRules = compileResult.badfilterRules;
  }
  if (typeof compileResult.badfilteredRules === 'number') {
    stats.badfilteredRules = compileResult.badfilteredRules;
  }

  snapshotStats = stats;

  await saveLists(updatedLists);

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

interface ResponseDetails extends RequestDetails {
  responseHeaders?: chrome.webRequest.HttpHeader[];
}

function onBeforeRequest(
  details: RequestDetails
): chrome.webRequest.BlockingResponse | undefined {
  traceMaybeRecord(details);
  const perfStart = performance.now();
  const finalize = (response?: chrome.webRequest.BlockingResponse) => {
    perfMaybeRecord('beforeRequest', performance.now() - perfStart);
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

  const dynamicMatch = matchDynamicRules(details, initiator);

  if (dynamicMatch.action === DynamicAction.BLOCK) {
    if (details.type === 'main_frame' && isOverlyBroadDynamicRule(dynamicMatch.rule)) {
      console.warn('[BetterBlocker] Skipping overly broad dynamic rule for main_frame', dynamicMatch.rule);
      return finalize(undefined);
    }
    incrementTabBlockCount(details.tabId);
    return finalize({ cancel: true });
  }

  if (dynamicMatch.action === DynamicAction.ALLOW) {
    return finalize(undefined);
  }

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
        incrementTabBlockCount(details.tabId);
        return finalize({ cancel: true });

      case MatchDecision.REDIRECT:
        if (result.redirectUrl) {
          incrementTabBlockCount(details.tabId);
          const redirectUrl = result.redirectUrl.startsWith('/')
            ? api.runtime.getURL(`resources${result.redirectUrl}`)
            : result.redirectUrl;
          return finalize({ redirectUrl });
        }
        incrementTabBlockCount(details.tabId);
        return finalize({ cancel: true });

      case MatchDecision.REMOVEPARAM:
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
    console.error('[BetterBlocker] Match error:', e);
    return finalize(undefined);
  }
}

function onHeadersReceived(
  details: ResponseDetails
): chrome.webRequest.BlockingResponse | undefined {
  const perfStart = performance.now();
  const finalize = (response?: chrome.webRequest.BlockingResponse) => {
    perfMaybeRecord('headersReceived', performance.now() - perfStart);
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
    const result = wasm.match_response_headers(
      details.url,
      details.type,
      initiator,
      details.tabId,
      details.frameId,
      details.requestId,
      headers
    );

    if (result.cancel) {
      incrementTabBlockCount(details.tabId);
      return finalize({ cancel: true });
    }

    const removeHeaders = settings.responseHeaderEnabled ? (result.removeHeaders ?? []) : [];
    let responseHeaders = headers;

    if (removeHeaders.length > 0) {
      const removeSet = new Set(removeHeaders.map((name) => name.toLowerCase()));
      responseHeaders = headers.filter((header) => !removeSet.has(header.name.toLowerCase()));
    }

    if (settings.cspEnabled && result.csp && result.csp.length > 0) {
      responseHeaders = [...responseHeaders];
      for (const value of result.csp) {
        responseHeaders.push({ name: 'Content-Security-Policy', value });
      }
      return finalize({ responseHeaders });
    }

    if (removeHeaders.length > 0) {
      return finalize({ responseHeaders });
    }

    return finalize(undefined);
  } catch (e) {
    console.error('[BetterBlocker] Header match error:', e);
    return finalize(undefined);
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

  api.webRequest.onHeadersReceived.addListener(
    onHeadersReceived as Parameters<typeof api.webRequest.onHeadersReceived.addListener>[0],
    filter,
    ['blocking', 'responseHeaders']
  );

  console.log('[BetterBlocker] webRequest listener registered');
}

function setupTabTracking(): void {
  api.tabs.onRemoved.addListener((tabId) => {
    topFrameByTab.delete(tabId);
    blockedByTab.delete(tabId);
    mainFrameRequestIdByTab.delete(tabId);
    clearRemoveparamHistory(tabId);
  });
}

interface MessageRequest {
  type: string;
  maxEntries?: number;
  rules?: DynamicRule[];
  url?: string;
  requestId?: string;
  tabId?: number;
  enabled?: boolean;
  settings?: Partial<UserSettings>;
}

function setupMessageHandlers(): void {
  api.runtime.onMessage.addListener(
    (
      message: MessageRequest,
      sender: chrome.runtime.MessageSender,
      sendResponse: (response: unknown) => void
    ) => {
      switch (message.type) {
        case 'getStats': {
          const tabId =
            typeof message.tabId === 'number'
              ? message.tabId
              : (sender.tab?.id ?? -1);
          const siteUrl =
            typeof message.url === 'string'
              ? message.url
              : (sender.tab?.url ?? topFrameByTab.get(tabId));
          const initialized = wasm?.is_initialized() ?? false;
          if (!initialized) {
            void maybeAutoCompile('getStats');
          }
          let snapshotInfo: { size: number; initialized: boolean } | null = null;
          try {
            snapshotInfo = wasm?.get_snapshot_info() ?? null;
          } catch (e) {
            console.warn('[BetterBlocker] Snapshot info error:', e);
          }
          sendResponse({
            blockCount,
            enabled: settings.enabled,
            initialized,
            snapshotInfo,
            snapshotStats,
            tabBlockCount: getTabBlockCount(tabId),
            siteDisabled: isSiteDisabled(siteUrl),
          });
          return true;
        }

        case 'dynamic.get':
          sendResponse({ rules: dynamicRules });
          return true;

        case 'dynamic.set': {
          const rules = Array.isArray(message.rules) ? message.rules : [];
          dynamicRules = rules;
          saveDynamicRules(rules)
            .then(() => sendResponse({ ok: true }))
            .catch((e: Error) => sendResponse({ ok: false, error: e.message }));
          return true;
        }

        case 'cosmetic.get': {
          const url =
            typeof message.url === 'string'
              ? message.url
              : (sender.url ?? sender.tab?.url);
          if (
            !url ||
            !wasm?.is_initialized() ||
            !settings.enabled ||
            isSiteDisabled(url) ||
            (!settings.cosmeticsEnabled && !settings.scriptletsEnabled)
          ) {
            sendResponse({ css: '', enableGeneric: true, procedural: [], scriptlets: [] });
            return true;
          }
          const tabId = sender.tab?.id ?? -1;
          const frameId = sender.frameId ?? 0;
          const requestId = message.requestId ?? 'cosmetic';
          let result: { css: string; enableGeneric: boolean; procedural: string[]; scriptlets: { name: string; args: string[] }[] };
          try {
            result = wasm.match_cosmetics(url, 'main_frame', undefined, tabId, frameId, requestId);
          } catch (e) {
            console.warn('[BetterBlocker] Cosmetic match error:', e);
            sendResponse({ css: '', enableGeneric: true, procedural: [], scriptlets: [] });
            return true;
          }
          if (!settings.cosmeticsEnabled) {
            result.css = '';
            result.enableGeneric = false;
            result.procedural = [];
          }
          if (!settings.scriptletsEnabled || frameId !== 0) {
            result.scriptlets = [];
          }
          sendResponse(result);
          return true;
        }

        case 'settings.get':
          sendResponse({ settings });
          return true;

        case 'settings.update': {
          const update = message.settings ?? {};
          const next = applySettings(update);
          saveSettings(next)
            .then(() => {
              updateAllBadges();
              sendResponse({ ok: true, settings: next });
            })
            .catch((e: Error) => sendResponse({ ok: false, error: e.message, settings }));
          return true;
        }

        case 'site.toggle': {
          const url =
            typeof message.url === 'string'
              ? message.url
              : (sender.tab?.url ?? sender.url);
          const enabledValue = typeof message.enabled === 'boolean' ? message.enabled : undefined;
          if (!url || typeof enabledValue !== 'boolean') {
            sendResponse({ ok: false, error: 'Missing url or enabled flag' });
            return true;
          }
          const pattern = getSitePattern(url);
          if (!pattern) {
            sendResponse({ ok: false, error: 'Invalid site url' });
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
          saveSettings(next)
            .then(() => {
              if (tabId >= 0) {
                updateBadge(tabId);
              }
              sendResponse({ ok: true, disabled: !enabledValue, sitePattern: pattern, settings: next });
            })
            .catch((e: Error) => sendResponse({ ok: false, error: e.message, settings }));
          return true;
        }

        case 'toggleEnabled': {
          const next = applySettings({ enabled: !settings.enabled });
          saveSettings(next)
            .then(() => {
              updateAllBadges();
              if (next.enabled) {
                api.browserAction.setIcon({ path: 'icons/icon48.png' });
              }
              sendResponse({ enabled: next.enabled });
            })
            .catch((e: Error) => sendResponse({ enabled: settings.enabled, error: e.message }));
          return true;
        }

        case 'updateList':
        case 'updateAllLists':
        case 'listsChanged':
          compileAndStoreLists()
            .then(({ stats }) => {
              sendResponse({ success: true, snapshotStats: stats });
            })
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

        case 'perf.start':
          perfConfigure(true, message.maxEntries ?? 100_000);
          sendResponse({ ok: true, stats: perfStats() });
          return true;

        case 'perf.stop':
          perfConfigure(false);
          sendResponse({ ok: true, stats: perfStats() });
          return true;

        case 'perf.stats':
          sendResponse({ ok: true, stats: perfStats() });
          return true;

        case 'perf.export':
          sendResponse({ ok: true, json: perfExportJson(), stats: perfStats() });
          return true;

        default:
          return false;
      }
    }
  );
}

initPromise = initialize();
setupWebRequest();
setupTabTracking();
setupMessageHandlers();

initPromise.catch((e) => {
  console.error('[BetterBlocker] Failed to initialize:', e);
});
