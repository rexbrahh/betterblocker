/// <reference types="chrome"/>

import { sendMessageStrict as sendMessage } from '../shared/messaging.js';

interface StatsResponse {
  blockCount: number;
  enabled: boolean;
  initialized: boolean;
  snapshotInfo: { size: number; initialized: boolean } | null;
}

const elements = {
  blockCount: document.getElementById('block-count') as HTMLElement,
  statusDot: document.querySelector('.status-dot') as HTMLElement,
  statusText: document.querySelector('.status-text') as HTMLElement,
  statusBadge: document.getElementById('status-indicator') as HTMLElement,
  toggle: document.getElementById('enabled-toggle') as HTMLInputElement,
};

function updateStatus(initialized: boolean, enabled: boolean) {
  if (!initialized) {
    elements.statusBadge.classList.remove('ready', 'disabled', 'error');
    elements.statusBadge.classList.add('loading');
    elements.statusText.textContent = 'Initializing...';
    elements.statusDot.style.backgroundColor = 'var(--warning-color)';
    return;
  }

  elements.statusBadge.classList.remove('loading', 'error');
  
  if (enabled) {
    elements.statusBadge.classList.remove('disabled');
    elements.statusBadge.classList.add('ready');
    elements.statusText.textContent = 'Active';
    elements.statusDot.style.backgroundColor = 'var(--success-color)';
  } else {
    elements.statusBadge.classList.remove('ready');
    elements.statusBadge.classList.add('disabled');
    elements.statusText.textContent = 'Disabled';
    elements.statusDot.style.backgroundColor = 'var(--text-secondary)';
  }
}

function updateStats(response: StatsResponse) {
  elements.blockCount.textContent = response.blockCount.toLocaleString();
  
  if (elements.toggle.checked !== response.enabled) {
    elements.toggle.checked = response.enabled;
  }
  
  elements.toggle.disabled = !response.initialized;
  
  updateStatus(response.initialized, response.enabled);
}

async function fetchStats() {
  try {
    const response = await sendMessage<StatsResponse>({ type: 'getStats' });
    updateStats(response);
  } catch (error) {
    console.error('Failed to fetch stats:', error);
    elements.statusBadge.classList.remove('ready', 'loading', 'disabled');
    elements.statusBadge.classList.add('error');
    elements.statusText.textContent = 'Error';
    elements.statusDot.style.backgroundColor = 'var(--danger-color)';
  }
}

elements.toggle.addEventListener('change', async (e) => {
  const target = e.target as HTMLInputElement;
  const newState = target.checked;
  
  updateStatus(true, newState); 

  try {
    await sendMessage({ type: 'toggleEnabled' });
    await fetchStats();
  } catch (error) {
    console.error('Failed to toggle:', error);
    target.checked = !newState;
    await fetchStats();
  }
});

fetchStats();

setInterval(fetchStats, 2000);
