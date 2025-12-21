(() => {
  // src/shared/messaging.ts
  function sendMessage(message) {
    return new Promise((resolve) => {
      chrome.runtime.sendMessage(message, (response) => {
        if (chrome.runtime.lastError) {
          console.warn("Message error:", chrome.runtime.lastError);
          resolve({});
        } else {
          resolve(response);
        }
      });
    });
  }

  // src/options/options.ts
  var STORAGE_KEY = "filterLists";
  var pageElements = {
    listContainer: document.getElementById("filter-list-container"),
    listCountBadge: document.getElementById("list-count-badge"),
    addForm: document.getElementById("add-list-form"),
    nameInput: document.getElementById("new-list-name"),
    urlInput: document.getElementById("new-list-url"),
    updateAllBtn: document.getElementById("update-all-btn"),
    totalRules: document.getElementById("total-rules-count"),
    extVersion: document.getElementById("ext-version"),
    traceStatusBadge: document.getElementById("trace-status-badge"),
    traceMaxEntries: document.getElementById("trace-max-entries"),
    traceStartBtn: document.getElementById("trace-start-btn"),
    traceStopBtn: document.getElementById("trace-stop-btn"),
    traceExportBtn: document.getElementById("trace-export-btn"),
    traceCount: document.getElementById("trace-count"),
    traceMax: document.getElementById("trace-max"),
    perfStatusBadge: document.getElementById("perf-status-badge"),
    perfMaxEntries: document.getElementById("perf-max-entries"),
    perfStartBtn: document.getElementById("perf-start-btn"),
    perfStopBtn: document.getElementById("perf-stop-btn"),
    perfExportBtn: document.getElementById("perf-export-btn"),
    perfBrCount: document.getElementById("perf-br-count"),
    perfBrMin: document.getElementById("perf-br-min"),
    perfBrP50: document.getElementById("perf-br-p50"),
    perfBrP95: document.getElementById("perf-br-p95"),
    perfBrP99: document.getElementById("perf-br-p99"),
    perfBrMax: document.getElementById("perf-br-max"),
    perfHrCount: document.getElementById("perf-hr-count"),
    perfHrMin: document.getElementById("perf-hr-min"),
    perfHrP50: document.getElementById("perf-hr-p50"),
    perfHrP95: document.getElementById("perf-hr-p95"),
    perfHrP99: document.getElementById("perf-hr-p99"),
    perfHrMax: document.getElementById("perf-hr-max"),
    toggles: {
      cosmeticsEnabled: document.getElementById("toggle-cosmeticsEnabled"),
      scriptletsEnabled: document.getElementById("toggle-scriptletsEnabled"),
      dynamicFilteringEnabled: document.getElementById("toggle-dynamicFilteringEnabled"),
      removeparamEnabled: document.getElementById("toggle-removeparamEnabled"),
      cspEnabled: document.getElementById("toggle-cspEnabled"),
      responseHeaderEnabled: document.getElementById("toggle-responseHeaderEnabled")
    }
  };
  function generateId() {
    return Date.now().toString(36) + Math.random().toString(36).substring(2);
  }
  function formatDate(dateString) {
    if (!dateString)
      return "Never";
    return new Date(dateString).toLocaleString();
  }
  async function getLists() {
    return new Promise((resolve) => {
      chrome.storage.sync.get([STORAGE_KEY], (result) => {
        const lists = result[STORAGE_KEY];
        resolve(lists || []);
      });
    });
  }
  async function saveLists(lists) {
    return new Promise((resolve) => {
      chrome.storage.sync.set({ [STORAGE_KEY]: lists }, () => {
        resolve();
      });
    });
  }
  function escapeHtml(text) {
    const div = document.createElement("div");
    div.textContent = text;
    return div.innerHTML;
  }
  async function toggleList(id, enabled) {
    const lists = await getLists();
    const list = lists.find((l) => l.id === id);
    if (list) {
      if (list.pinned && !enabled) {
        alert("Pinned lists cannot be disabled.");
        return;
      }
      list.enabled = enabled;
      await saveLists(lists);
      await sendMessage({ type: "listsChanged" });
    }
  }
  async function removeList(id) {
    const lists = await getLists();
    const list = lists.find((l) => l.id === id);
    if (list?.pinned) {
      alert("Pinned lists cannot be removed.");
      return;
    }
    if (!confirm("Are you sure you want to remove this filter list?"))
      return;
    const updatedLists = lists.filter((l) => l.id !== id);
    await saveLists(updatedLists);
    renderLists(updatedLists);
    await sendMessage({ type: "listsChanged" });
  }
  function renderLists(lists) {
    pageElements.listContainer.innerHTML = "";
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
      const item = document.createElement("div");
      item.className = "filter-list-item";
      const toggleId = `toggle-${list.id}`;
      const version = list.version ? `<span>Version: ${escapeHtml(list.version)}</span>` : "";
      const pinned = list.pinned ? "<span>Pinned</span>" : "";
      const homepage = list.homepage ? `<a href="${escapeHtml(list.homepage)}" target="_blank" rel="noopener">Homepage</a>` : "";
      const license = list.license ? `<a href="${escapeHtml(list.license)}" target="_blank" rel="noopener">License</a>` : "";
      const isPinned = !!list.pinned;
      const toggleDisabled = isPinned && list.enabled ? "disabled" : "";
      const removeDisabled = isPinned ? "disabled" : "";
      item.innerHTML = `
      <div class="filter-info">
        <span class="filter-name">${escapeHtml(list.name)}</span>
        <span class="filter-url">${escapeHtml(list.url)}</span>
        <div class="filter-meta">
          <span>Rules: ${list.ruleCount.toLocaleString()}</span>
          <span>Updated: ${formatDate(list.lastUpdated)}</span>
          ${version}
          ${pinned}
          ${homepage}
          ${license}
        </div>
      </div>
      <div class="filter-actions">
        <label class="toggle-switch">
          <input type="checkbox" id="${toggleId}" ${list.enabled ? "checked" : ""} ${toggleDisabled}>
          <span class="slider"></span>
        </label>
        <button class="btn danger-text remove-btn" data-id="${list.id}" aria-label="Remove List" ${removeDisabled}>
          Remove
        </button>
      </div>
    `;
      pageElements.listContainer.appendChild(item);
      const toggle = item.querySelector(`#${toggleId}`);
      toggle.addEventListener("change", () => toggleList(list.id, toggle.checked));
      const removeBtn = item.querySelector(".remove-btn");
      removeBtn.addEventListener("click", () => removeList(list.id));
    });
  }
  async function addList(name, url) {
    const lists = await getLists();
    const newList = {
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
    const response = await sendMessage({
      type: "updateList",
      payload: { id: newList.id, url: newList.url }
    });
    if (response?.success === false) {
      console.error("List compile failed", response.error);
      alert(response.error || "Failed to compile list");
    }
  }
  async function updateAllLists() {
    const btn = pageElements.updateAllBtn;
    const originalText = btn.innerHTML;
    btn.disabled = true;
    btn.innerHTML = '<span class="icon">⌛</span> Updating...';
    try {
      const response = await sendMessage({ type: "updateAllLists" });
      if (response?.success === false) {
        throw new Error(response.error || "Failed to compile lists");
      }
      const lists = await getLists();
      renderLists(lists);
    } catch (error) {
      console.error("Update failed", error);
      alert("Failed to update lists");
    } finally {
      btn.disabled = false;
      btn.innerHTML = originalText;
    }
  }
  async function loadStats() {
    const manifest = chrome.runtime.getManifest();
    pageElements.extVersion.textContent = `v${manifest.version}`;
    try {
      const stats = await sendMessage({ type: "getStats" });
      const ruleCount = stats?.snapshotStats?.rulesAfter;
      if (typeof ruleCount === "number") {
        pageElements.totalRules.textContent = ruleCount.toLocaleString();
      } else {
        pageElements.totalRules.textContent = "0";
      }
    } catch (e) {
      console.warn("Failed to load stats", e);
    }
  }
  function updateTraceUI(stats) {
    pageElements.traceStatusBadge.textContent = stats.enabled ? "Recording" : "Disabled";
    pageElements.traceStatusBadge.style.backgroundColor = stats.enabled ? "var(--success-color)" : "";
    pageElements.traceStatusBadge.style.color = stats.enabled ? "white" : "";
    pageElements.traceCount.textContent = stats.count.toLocaleString();
    pageElements.traceMax.textContent = stats.max.toLocaleString();
    pageElements.traceStartBtn.disabled = stats.enabled;
    pageElements.traceStopBtn.disabled = !stats.enabled;
    pageElements.traceExportBtn.disabled = stats.count === 0;
  }
  async function getTraceStats() {
    try {
      const response = await sendMessage({ type: "trace.stats" });
      if (response && response.stats) {
        updateTraceUI(response.stats);
      }
    } catch (e) {
      console.warn("Failed to get trace stats", e);
    }
  }
  async function startTrace() {
    const maxEntriesInput = pageElements.traceMaxEntries.value;
    const parsedMaxEntries = maxEntriesInput ? parseInt(maxEntriesInput, 10) : undefined;
    const message = { type: "trace.start" };
    if (typeof parsedMaxEntries === "number" && Number.isFinite(parsedMaxEntries)) {
      message.maxEntries = parsedMaxEntries;
    }
    try {
      const response = await sendMessage(message);
      if (response && response.stats) {
        updateTraceUI(response.stats);
      }
    } catch (e) {
      console.error("Failed to start trace", e);
    }
  }
  async function stopTrace() {
    try {
      const response = await sendMessage({ type: "trace.stop" });
      if (response && response.stats) {
        updateTraceUI(response.stats);
      }
    } catch (e) {
      console.error("Failed to stop trace", e);
    }
  }
  async function exportTrace() {
    const btn = pageElements.traceExportBtn;
    const originalText = btn.textContent;
    btn.disabled = true;
    btn.textContent = "Exporting...";
    try {
      const response = await sendMessage({ type: "trace.export" });
      if (response && response.ok && response.jsonl) {
        const blob = new Blob([response.jsonl], { type: "application/x-jsonlines" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = "trace.jsonl";
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      }
      if (response && response.stats) {
        updateTraceUI(response.stats);
      }
    } catch (e) {
      console.error("Failed to export trace", e);
      alert("Failed to export trace");
    } finally {
      btn.disabled = false;
      btn.textContent = originalText || "Export JSONL";
    }
  }
  function updatePerfUI(stats) {
    pageElements.perfStatusBadge.textContent = stats.enabled ? "Recording" : "Disabled";
    pageElements.perfStatusBadge.style.backgroundColor = stats.enabled ? "var(--success-color)" : "";
    pageElements.perfStatusBadge.style.color = stats.enabled ? "white" : "";
    pageElements.perfStartBtn.disabled = stats.enabled;
    pageElements.perfStopBtn.disabled = !stats.enabled;
    const updateBucket = (bucket, prefix) => {
      const hasData = bucket.count > 0;
      if (prefix === "perfBr") {
        pageElements.perfBrCount.textContent = bucket.count.toLocaleString();
        pageElements.perfBrMin.textContent = hasData ? bucket.min.toLocaleString() : "-";
        pageElements.perfBrP50.textContent = hasData ? bucket.p50.toLocaleString() : "-";
        pageElements.perfBrP95.textContent = hasData ? bucket.p95.toLocaleString() : "-";
        pageElements.perfBrP99.textContent = hasData ? bucket.p99.toLocaleString() : "-";
        pageElements.perfBrMax.textContent = hasData ? bucket.max.toLocaleString() : "-";
      } else {
        pageElements.perfHrCount.textContent = bucket.count.toLocaleString();
        pageElements.perfHrMin.textContent = hasData ? bucket.min.toLocaleString() : "-";
        pageElements.perfHrP50.textContent = hasData ? bucket.p50.toLocaleString() : "-";
        pageElements.perfHrP95.textContent = hasData ? bucket.p95.toLocaleString() : "-";
        pageElements.perfHrP99.textContent = hasData ? bucket.p99.toLocaleString() : "-";
        pageElements.perfHrMax.textContent = hasData ? bucket.max.toLocaleString() : "-";
      }
    };
    updateBucket(stats.beforeRequest, "perfBr");
    updateBucket(stats.headersReceived, "perfHr");
    pageElements.perfExportBtn.disabled = stats.beforeRequest.count === 0 && stats.headersReceived.count === 0;
  }
  async function getPerfStats() {
    try {
      const response = await sendMessage({ type: "perf.stats" });
      if (response && response.stats) {
        updatePerfUI(response.stats);
      }
    } catch (e) {
      console.warn("Failed to get perf stats", e);
    }
  }
  async function startPerf() {
    const maxEntriesInput = pageElements.perfMaxEntries.value;
    const parsedMaxEntries = maxEntriesInput ? parseInt(maxEntriesInput, 10) : undefined;
    const message = { type: "perf.start" };
    if (typeof parsedMaxEntries === "number" && Number.isFinite(parsedMaxEntries)) {
      message.maxEntries = parsedMaxEntries;
    }
    try {
      const response = await sendMessage(message);
      if (response && response.stats) {
        updatePerfUI(response.stats);
      }
    } catch (e) {
      console.error("Failed to start perf", e);
    }
  }
  async function stopPerf() {
    try {
      const response = await sendMessage({ type: "perf.stop" });
      if (response && response.stats) {
        updatePerfUI(response.stats);
      }
    } catch (e) {
      console.error("Failed to stop perf", e);
    }
  }
  async function exportPerf() {
    const btn = pageElements.perfExportBtn;
    const originalText = btn.textContent;
    btn.disabled = true;
    btn.textContent = "Exporting...";
    try {
      const response = await sendMessage({ type: "perf.export" });
      if (response && response.ok && response.json) {
        const blob = new Blob([response.json], { type: "application/json" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = "perf.json";
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      }
      if (response && response.stats) {
        updatePerfUI(response.stats);
      }
    } catch (e) {
      console.error("Failed to export perf", e);
      alert("Failed to export perf data");
    } finally {
      btn.disabled = false;
      btn.textContent = originalText || "Export JSON";
    }
  }
  async function loadSettings() {
    try {
      const response = await sendMessage({ type: "settings.get" });
      const settings = response?.settings;
      if (!settings)
        return;
      for (const [key, element] of Object.entries(pageElements.toggles)) {
        if (key in settings) {
          const value = settings[key];
          element.checked = Boolean(value);
        }
      }
    } catch (e) {
      console.error("Failed to load settings", e);
    }
  }
  async function updateSetting(key, value) {
    try {
      await sendMessage({
        type: "settings.update",
        settings: { [key]: value }
      });
    } catch (e) {
      console.error("Failed to update setting", e);
      if (key in pageElements.toggles) {
        const el = pageElements.toggles[key];
        el.checked = !value;
      }
    }
  }
  async function init() {
    const lists = await getLists();
    renderLists(lists);
    loadStats();
    getTraceStats();
    getPerfStats();
    loadSettings();
    pageElements.traceStartBtn.addEventListener("click", startTrace);
    pageElements.traceStopBtn.addEventListener("click", stopTrace);
    pageElements.traceExportBtn.addEventListener("click", exportTrace);
    pageElements.perfStartBtn.addEventListener("click", startPerf);
    pageElements.perfStopBtn.addEventListener("click", stopPerf);
    pageElements.perfExportBtn.addEventListener("click", exportPerf);
    for (const [key, element] of Object.entries(pageElements.toggles)) {
      element.addEventListener("change", (e) => {
        updateSetting(key, e.target.checked);
      });
    }
    pageElements.addForm.addEventListener("submit", async (e) => {
      e.preventDefault();
      const name = pageElements.nameInput.value.trim();
      const url = pageElements.urlInput.value.trim();
      if (name && url) {
        await addList(name, url);
        pageElements.addForm.reset();
      }
    });
    pageElements.updateAllBtn.addEventListener("click", updateAllLists);
    chrome.storage.onChanged.addListener((changes, area) => {
      if (area === "sync" && changes[STORAGE_KEY]) {
        renderLists(changes[STORAGE_KEY].newValue);
      }
    });
  }
  init();
})();
