(() => {
  // src/shared/messaging.ts
  function sendMessageStrict(message) {
    return new Promise((resolve, reject) => {
      chrome.runtime.sendMessage(message, (response) => {
        if (chrome.runtime.lastError) {
          reject(chrome.runtime.lastError);
        } else {
          resolve(response);
        }
      });
    });
  }

  // src/popup/popup.ts
  var elements = {
    blockCount: document.getElementById("block-count"),
    statusDot: document.querySelector(".status-dot"),
    statusText: document.querySelector(".status-text"),
    statusBadge: document.getElementById("status-indicator"),
    toggle: document.getElementById("enabled-toggle"),
    siteToggle: document.getElementById("site-toggle"),
    siteSection: document.getElementById("site-section"),
    siteHostname: document.getElementById("site-hostname"),
    siteFavicon: document.getElementById("site-favicon"),
    siteBlockCount: document.getElementById("site-block-count")
  };
  var currentTabId;
  var currentUrl;
  async function getCurrentTab() {
    const tabs = await chrome.tabs.query({ active: true, currentWindow: true });
    return tabs[0];
  }
  function updateStatus(initialized, enabled) {
    if (!initialized) {
      elements.statusBadge.classList.remove("ready", "disabled", "error");
      elements.statusBadge.classList.add("loading");
      elements.statusText.textContent = "Initializing...";
      elements.statusDot.style.backgroundColor = "var(--warning-color)";
      return;
    }
    elements.statusBadge.classList.remove("loading", "error");
    if (enabled) {
      elements.statusBadge.classList.remove("disabled");
      elements.statusBadge.classList.add("ready");
      elements.statusText.textContent = "Active";
      elements.statusDot.style.backgroundColor = "var(--success-color)";
    } else {
      elements.statusBadge.classList.remove("ready");
      elements.statusBadge.classList.add("disabled");
      elements.statusText.textContent = "Disabled";
      elements.statusDot.style.backgroundColor = "var(--text-secondary)";
    }
  }
  function updateStats(response) {
    elements.blockCount.textContent = response.blockCount.toLocaleString();
    if (elements.toggle.checked !== response.enabled) {
      elements.toggle.checked = response.enabled;
    }
    elements.toggle.disabled = !response.initialized;
    if (response.tabBlockCount !== undefined) {
      elements.siteBlockCount.textContent = response.tabBlockCount.toLocaleString();
    }
    if (response.siteDisabled !== undefined) {
      elements.siteToggle.checked = !response.siteDisabled;
      elements.siteToggle.disabled = !response.initialized || !response.enabled;
    }
    updateStatus(response.initialized, response.enabled);
  }
  async function fetchStats() {
    try {
      const response = await sendMessageStrict({
        type: "getStats",
        tabId: currentTabId,
        url: currentUrl
      });
      updateStats(response);
    } catch (error) {
      console.error("Failed to fetch stats:", error);
      elements.statusBadge.classList.remove("ready", "loading", "disabled");
      elements.statusBadge.classList.add("error");
      elements.statusText.textContent = "Error";
      elements.statusDot.style.backgroundColor = "var(--danger-color)";
    }
  }
  async function init() {
    const tab = await getCurrentTab();
    if (tab) {
      currentTabId = tab.id;
      currentUrl = tab.url;
      if (tab.url) {
        try {
          const urlObj = new URL(tab.url);
          elements.siteHostname.textContent = urlObj.hostname;
          if (tab.favIconUrl) {
            elements.siteFavicon.src = tab.favIconUrl;
          } else {
            elements.siteFavicon.style.display = "none";
          }
        } catch (e) {
          elements.siteSection.style.display = "none";
        }
      } else {
        elements.siteSection.style.display = "none";
      }
    }
    fetchStats();
    setInterval(fetchStats, 2000);
  }
  elements.toggle.addEventListener("change", async (e) => {
    const target = e.target;
    const newState = target.checked;
    updateStatus(true, newState);
    try {
      await sendMessageStrict({ type: "toggleEnabled" });
      await fetchStats();
    } catch (error) {
      console.error("Failed to toggle:", error);
      target.checked = !newState;
      await fetchStats();
    }
  });
  elements.siteToggle.addEventListener("change", async (e) => {
    if (!currentUrl)
      return;
    const target = e.target;
    const newState = target.checked;
    try {
      await sendMessageStrict({
        type: "site.toggle",
        url: currentUrl,
        enabled: newState,
        tabId: currentTabId
      });
      await fetchStats();
    } catch (error) {
      console.error("Failed to toggle site:", error);
      target.checked = !newState;
    }
  });
  init();
})();
