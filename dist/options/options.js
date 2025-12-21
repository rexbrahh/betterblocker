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
    extVersion: document.getElementById("ext-version")
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
      list.enabled = enabled;
      await saveLists(lists);
      await sendMessage({ type: "listsChanged" });
    }
  }
  async function removeList(id) {
    if (!confirm("Are you sure you want to remove this filter list?"))
      return;
    const lists = await getLists();
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
          <input type="checkbox" id="${toggleId}" ${list.enabled ? "checked" : ""}>
          <span class="slider"></span>
        </label>
        <button class="btn danger-text remove-btn" data-id="${list.id}" aria-label="Remove List">
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
    await sendMessage({ type: "updateList", payload: { id: newList.id, url: newList.url } });
  }
  async function updateAllLists() {
    const btn = pageElements.updateAllBtn;
    const originalText = btn.innerHTML;
    btn.disabled = true;
    btn.innerHTML = '<span class="icon">⌛</span> Updating...';
    try {
      await sendMessage({ type: "updateAllLists" });
      await new Promise((r) => setTimeout(r, 2000));
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
      if (stats && stats.snapshotInfo) {
        pageElements.totalRules.textContent = stats.snapshotInfo.size.toLocaleString();
      } else {
        pageElements.totalRules.textContent = "0";
      }
    } catch (e) {
      console.warn("Failed to load stats", e);
    }
  }
  async function init() {
    const lists = await getLists();
    renderLists(lists);
    loadStats();
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
