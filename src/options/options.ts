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
  
  await sendMessage({ type: 'listsChanged' });
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
  
  const response = await sendMessage<{ success?: boolean; error?: string }>({
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
    const response = await sendMessage<{ success?: boolean; error?: string }>({ type: 'updateAllLists' });
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
    const stats = await sendMessage<StatsResponse>({ type: 'getStats' });
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

async function init() {
  const lists = await getLists();
  renderLists(lists);
  loadStats();

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
