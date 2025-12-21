/// <reference types="chrome"/>

import { sendMessage } from '../shared/messaging.js';

interface FilterList {
  id: string;
  name: string;
  url: string;
  enabled: boolean;
  ruleCount: number;
  lastUpdated: string | null;
}

interface SnapshotStats {
  rulesBefore: number;
  rulesAfter: number;
  listStats: { lines: number; rulesBefore: number; rulesAfter: number }[];
}

interface StatsResponse {
  blockCount: number;
  enabled: boolean;
  initialized: boolean;
  snapshotInfo: { size: number; initialized: boolean } | null;
  snapshotStats?: SnapshotStats | null;
}

interface TraceStats {
  enabled: boolean;
  count: number;
  max: number;
}

interface TraceResponse {
  ok: boolean;
  stats: TraceStats;
  jsonl?: string;
}

type BgMessage = {
  type: string;
  payload?: unknown;
  maxEntries?: number;
};

interface UserSettings {
  enabled: boolean;
  cosmeticsEnabled: boolean;
  scriptletsEnabled: boolean;
  dynamicFilteringEnabled: boolean;
  removeparamEnabled: boolean;
  cspEnabled: boolean;
  responseHeaderEnabled: boolean;
  disabledSites: string[];
}

const STORAGE_KEY = 'filterLists';

const pageElements = {
  listContainer: document.getElementById('filter-list-container') as HTMLElement,
  listCountBadge: document.getElementById('list-count-badge') as HTMLElement,
  addForm: document.getElementById('add-list-form') as HTMLFormElement,
  nameInput: document.getElementById('new-list-name') as HTMLInputElement,
  urlInput: document.getElementById('new-list-url') as HTMLInputElement,
  updateAllBtn: document.getElementById('update-all-btn') as HTMLButtonElement,
  totalRules: document.getElementById('total-rules-count') as HTMLElement,
  extVersion: document.getElementById('ext-version') as HTMLElement,
  traceStatusBadge: document.getElementById('trace-status-badge') as HTMLElement,
  traceMaxEntries: document.getElementById('trace-max-entries') as HTMLInputElement,
  traceStartBtn: document.getElementById('trace-start-btn') as HTMLButtonElement,
  traceStopBtn: document.getElementById('trace-stop-btn') as HTMLButtonElement,
  traceExportBtn: document.getElementById('trace-export-btn') as HTMLButtonElement,
  traceCount: document.getElementById('trace-count') as HTMLElement,
  traceMax: document.getElementById('trace-max') as HTMLElement,
  toggles: {
    cosmeticsEnabled: document.getElementById('toggle-cosmeticsEnabled') as HTMLInputElement,
    scriptletsEnabled: document.getElementById('toggle-scriptletsEnabled') as HTMLInputElement,
    dynamicFilteringEnabled: document.getElementById('toggle-dynamicFilteringEnabled') as HTMLInputElement,
    removeparamEnabled: document.getElementById('toggle-removeparamEnabled') as HTMLInputElement,
    cspEnabled: document.getElementById('toggle-cspEnabled') as HTMLInputElement,
    responseHeaderEnabled: document.getElementById('toggle-responseHeaderEnabled') as HTMLInputElement,
  }
};

function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).substring(2);
}

function formatDate(dateString: string | null): string {
  if (!dateString) return 'Never';
  return new Date(dateString).toLocaleString();
}

async function getLists(): Promise<FilterList[]> {
  return new Promise((resolve) => {
    chrome.storage.sync.get([STORAGE_KEY], (result) => {
      const lists = result[STORAGE_KEY] as FilterList[] | undefined;
      resolve(lists || []);
    });
  });
}

async function saveLists(lists: FilterList[]): Promise<void> {
  return new Promise((resolve) => {
    chrome.storage.sync.set({ [STORAGE_KEY]: lists }, () => {
      resolve();
    });
  });
}

function escapeHtml(text: string): string {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

async function toggleList(id: string, enabled: boolean) {
  const lists = await getLists();
  const list = lists.find(l => l.id === id);
  
  if (list) {
    list.enabled = enabled;
    await saveLists(lists);
    await sendMessage({ type: 'listsChanged' });
  }
}

async function removeList(id: string) {
  if (!confirm('Are you sure you want to remove this filter list?')) return;

  const lists = await getLists();
  const updatedLists = lists.filter(l => l.id !== id);
  
  await saveLists(updatedLists);
  renderLists(updatedLists);
  
  await sendMessage<void, BgMessage>({ type: 'listsChanged' });
}

function renderLists(lists: FilterList[]) {
  pageElements.listContainer.innerHTML = '';
  pageElements.listCountBadge.textContent = `${lists.length} lists`;

  if (lists.length === 0) {
    pageElements.listContainer.innerHTML = `
      <div class="empty-state">
        <p>No filter lists configured.</p>
      </div>
    `;
    return;
  }

  lists.forEach((list) => {
    const item = document.createElement('div');
    item.className = 'filter-list-item';
    
    const toggleId = `toggle-${list.id}`;

    item.innerHTML = `
      <div class="filter-info">
        <span class="filter-name">${escapeHtml(list.name)}</span>
        <span class="filter-url">${escapeHtml(list.url)}</span>
        <div class="filter-meta">
          <span>Rules: ${list.ruleCount.toLocaleString()}</span>
          <span>Updated: ${formatDate(list.lastUpdated)}</span>
        </div>
      </div>
      <div class="filter-actions">
        <label class="toggle-switch">
          <input type="checkbox" id="${toggleId}" ${list.enabled ? 'checked' : ''}>
          <span class="slider"></span>
        </label>
        <button class="btn danger-text remove-btn" data-id="${list.id}" aria-label="Remove List">
          Remove
        </button>
      </div>
    `;

    pageElements.listContainer.appendChild(item);

    const toggle = item.querySelector(`#${toggleId}`) as HTMLInputElement;
    toggle.addEventListener('change', () => toggleList(list.id, toggle.checked));

    const removeBtn = item.querySelector('.remove-btn') as HTMLButtonElement;
    removeBtn.addEventListener('click', () => removeList(list.id));
  });
}

async function addList(name: string, url: string) {
  const lists = await getLists();
  const newList: FilterList = {
    id: generateId(),
    name,
    url,
    enabled: true,
    ruleCount: 0,
    lastUpdated: null
  };
  
  lists.push(newList);
  await saveLists(lists);
  renderLists(lists);
  
  const response: { success?: boolean; error?: string } = await sendMessage({
    type: 'updateList',
    payload: { id: newList.id, url: newList.url },
  });
  if (response?.success === false) {
    console.error('List compile failed', response.error);
    alert(response.error || 'Failed to compile list');
  }
}

async function updateAllLists() {
  const btn = pageElements.updateAllBtn;
  const originalText = btn.innerHTML;
  
  btn.disabled = true;
  btn.innerHTML = '<span class="icon">⌛</span> Updating...';
  
  try {
    const response: { success?: boolean; error?: string } = await sendMessage({ type: 'updateAllLists' });
    if (response?.success === false) {
      throw new Error(response.error || 'Failed to compile lists');
    }

    const lists = await getLists();
    renderLists(lists);
  } catch (error) {
    console.error('Update failed', error);
    alert('Failed to update lists');
  } finally {
    btn.disabled = false;
    btn.innerHTML = originalText;
  }
}

async function loadStats() {
  const manifest = chrome.runtime.getManifest();
  pageElements.extVersion.textContent = `v${manifest.version}`;

  try {
    const stats: StatsResponse = await sendMessage({ type: 'getStats' });
    const ruleCount = stats?.snapshotStats?.rulesAfter;
    if (typeof ruleCount === 'number') {
      pageElements.totalRules.textContent = ruleCount.toLocaleString();
    } else {
      pageElements.totalRules.textContent = '0';
    }
  } catch (e) {
    console.warn('Failed to load stats', e);
  }
}

function updateTraceUI(stats: TraceStats) {
  pageElements.traceStatusBadge.textContent = stats.enabled ? 'Recording' : 'Disabled';
  pageElements.traceStatusBadge.style.backgroundColor = stats.enabled ? 'var(--success-color)' : '';
  pageElements.traceStatusBadge.style.color = stats.enabled ? 'white' : '';
  
  pageElements.traceCount.textContent = stats.count.toLocaleString();
  pageElements.traceMax.textContent = stats.max.toLocaleString();
  
  pageElements.traceStartBtn.disabled = stats.enabled;
  pageElements.traceStopBtn.disabled = !stats.enabled;
  pageElements.traceExportBtn.disabled = stats.count === 0;
}

async function getTraceStats() {
  try {
    const response: TraceResponse = await sendMessage({ type: 'trace.stats' });
    if (response && response.stats) {
      updateTraceUI(response.stats);
    }
  } catch (e) {
    console.warn('Failed to get trace stats', e);
  }
}

async function startTrace() {
  const maxEntriesInput = pageElements.traceMaxEntries.value;
  const parsedMaxEntries = maxEntriesInput ? parseInt(maxEntriesInput, 10) : undefined;
  const message: BgMessage = { type: 'trace.start' };
  if (typeof parsedMaxEntries === 'number' && Number.isFinite(parsedMaxEntries)) {
    message.maxEntries = parsedMaxEntries;
  }

  try {
    const response: TraceResponse = await sendMessage(message);
    if (response && response.stats) {
      updateTraceUI(response.stats);
    }
  } catch (e) {
    console.error('Failed to start trace', e);
  }
}

async function stopTrace() {
  try {
    const response: TraceResponse = await sendMessage({ type: 'trace.stop' });
    if (response && response.stats) {
      updateTraceUI(response.stats);
    }
  } catch (e) {
    console.error('Failed to stop trace', e);
  }
}

async function exportTrace() {
  const btn = pageElements.traceExportBtn;
  const originalText = btn.textContent;
  btn.disabled = true;
  btn.textContent = 'Exporting...';

  try {
    const response: TraceResponse = await sendMessage({ type: 'trace.export' });
    if (response && response.ok && response.jsonl) {
      const blob = new Blob([response.jsonl], { type: 'application/x-jsonlines' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'trace.jsonl';
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    }
    
    if (response && response.stats) {
        updateTraceUI(response.stats);
    }
  } catch (e) {
    console.error('Failed to export trace', e);
    alert('Failed to export trace');
  } finally {
    btn.disabled = false;
    btn.textContent = originalText || 'Export JSONL';
  }
}

async function loadSettings() {
  try {
    const response: { settings?: UserSettings } = await sendMessage({ type: 'settings.get' });
    const settings = response?.settings;
    if (!settings) return;

    for (const [key, element] of Object.entries(pageElements.toggles)) {
      if (key in settings) {
        const value = (settings as unknown as Record<string, unknown>)[key];
        element.checked = Boolean(value);
      }
    }
  } catch (e) {
    console.error('Failed to load settings', e);
  }
}

async function updateSetting(key: string, value: boolean) {
  try {
    await sendMessage({
      type: 'settings.update',
      settings: { [key]: value },
    });
  } catch (e) {
    console.error('Failed to update setting', e);
    if (key in pageElements.toggles) {
      const el = pageElements.toggles[key as keyof typeof pageElements.toggles];
      el.checked = !value;
    }
  }
}

async function init() {
  const lists = await getLists();
  renderLists(lists);
  loadStats();
  getTraceStats();
  loadSettings();

  pageElements.traceStartBtn.addEventListener('click', startTrace);
  pageElements.traceStopBtn.addEventListener('click', stopTrace);
  pageElements.traceExportBtn.addEventListener('click', exportTrace);

  for (const [key, element] of Object.entries(pageElements.toggles)) {
    element.addEventListener('change', (e) => {
      updateSetting(key, (e.target as HTMLInputElement).checked);
    });
  }

  pageElements.addForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const name = pageElements.nameInput.value.trim();
    const url = pageElements.urlInput.value.trim();
    
    if (name && url) {
      await addList(name, url);
      pageElements.addForm.reset();
    }
  });

  pageElements.updateAllBtn.addEventListener('click', updateAllLists);
  
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === 'sync' && changes[STORAGE_KEY]) {
      renderLists(changes[STORAGE_KEY].newValue as FilterList[]);
    }
  });
}

init();
