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

  // src/cs/scriptlets.ts
  var MAX_SELECTOR_MATCHES = 200;
  function parseValue(raw) {
    if (raw === undefined) {
      return;
    }
    const trimmed = raw.trim();
    if (trimmed === "null") {
      return null;
    }
    if (trimmed === "true") {
      return true;
    }
    if (trimmed === "false") {
      return false;
    }
    if (trimmed === "undefined") {
      return;
    }
    if (trimmed === "") {
      return "";
    }
    const numeric = Number(trimmed);
    if (!Number.isNaN(numeric) && String(numeric) === trimmed) {
      return numeric;
    }
    return raw;
  }
  function setConstant(args) {
    const path = args[0];
    if (!path) {
      return;
    }
    const value = parseValue(args[1]);
    const parts = path.split(".").filter((part) => part.length > 0);
    if (parts.length === 0) {
      return;
    }
    let target = window;
    for (let i = 0;i < parts.length - 1; i++) {
      const key = parts[i];
      if (!key) {
        return;
      }
      const next = target[key];
      if (typeof next !== "object" || next === null) {
        return;
      }
      target = next;
    }
    const prop = parts[parts.length - 1];
    if (!prop) {
      return;
    }
    try {
      Object.defineProperty(target, prop, {
        value,
        configurable: true,
        writable: false
      });
    } catch (e) {
      try {
        target[prop] = value;
      } catch (err) {}
    }
  }
  function forEachNode(selector, fn) {
    if (!selector) {
      return;
    }
    let nodes = [];
    try {
      nodes = Array.from(document.querySelectorAll(selector));
    } catch (e) {
      return;
    }
    const limit = Math.min(nodes.length, MAX_SELECTOR_MATCHES);
    for (let i = 0;i < limit; i++) {
      const node = nodes[i];
      if (!node) {
        continue;
      }
      fn(node);
    }
  }
  function removeAttr(args) {
    const selector = args[0];
    const attr = args[1];
    if (!selector || !attr) {
      return;
    }
    forEachNode(selector, (node) => node.removeAttribute(attr));
  }
  function removeClass(args) {
    const selector = args[0];
    const className = args[1];
    if (!selector || !className) {
      return;
    }
    forEachNode(selector, (node) => {
      if (node instanceof HTMLElement) {
        node.classList.remove(className);
      }
    });
  }
  function addClass(args) {
    const selector = args[0];
    const className = args[1];
    if (!selector || !className) {
      return;
    }
    forEachNode(selector, (node) => {
      if (node instanceof HTMLElement) {
        node.classList.add(className);
      }
    });
  }
  function hideBySelector(args) {
    const selector = args[0];
    if (!selector) {
      return;
    }
    forEachNode(selector, (node) => {
      if (node instanceof HTMLElement) {
        node.style.setProperty("display", "none", "important");
      }
    });
  }
  function removeBySelector(args) {
    const selector = args[0];
    if (!selector) {
      return;
    }
    forEachNode(selector, (node) => {
      node.remove();
    });
  }
  var scriptlets = {
    "set-constant": setConstant,
    "remove-attr": removeAttr,
    "remove-class": removeClass,
    "add-class": addClass,
    "hide-by-selector": hideBySelector,
    "remove-by-selector": removeBySelector
  };
  var pageContextScriptlets = new Set(["set-constant"]);

  // src/cs/bootstrap.ts
  var MAX_SCRIPTLETS = 32;
  var MAX_SCRIPTLET_ARGS = 8;
  var MAX_PROCEDURAL_RULES = 64;
  var MAX_PROCEDURAL_NODES = 200;
  function isTopFrame() {
    try {
      return window.top === window;
    } catch (e) {
      return false;
    }
  }
  function injectPageScriptlets(calls) {
    if (!calls || calls.length === 0) {
      return;
    }
    const limited = calls.slice(0, MAX_SCRIPTLETS).map((call) => ({
      name: call.name,
      args: Array.isArray(call.args) ? call.args.slice(0, MAX_SCRIPTLET_ARGS) : []
    }));
    const root = document.documentElement;
    if (!root) {
      return;
    }
    try {
      root.setAttribute("data-bb-scriptlets", JSON.stringify(limited));
    } catch (e) {}
    const script = document.createElement("script");
    script.src = chrome.runtime.getURL("resources/scriptlets/registry.js");
    script.async = false;
    script.onload = () => script.remove();
    script.onerror = () => script.remove();
    (document.head || root).appendChild(script);
    try {
      document.dispatchEvent(new CustomEvent("bb-scriptlets", { detail: limited }));
    } catch (e) {}
  }
  var PROCEDURAL_TOKENS = [
    { type: "has-text", token: ":has-text(" },
    { type: "matches-css", token: ":matches-css(" },
    { type: "xpath", token: ":xpath(" },
    { type: "upward", token: ":upward(" },
    { type: "remove", token: ":remove(" },
    { type: "style", token: ":style(" }
  ];
  function findNextProceduralOp(raw, start) {
    let best = null;
    for (const entry of PROCEDURAL_TOKENS) {
      const idx = raw.indexOf(entry.token, start);
      if (idx === -1) {
        continue;
      }
      if (!best || idx < best.index) {
        best = { type: entry.type, token: entry.token, index: idx };
      }
    }
    return best;
  }
  function readParenContent(raw, start) {
    if (raw[start] !== "(") {
      return null;
    }
    let depth = 0;
    for (let i = start;i < raw.length; i++) {
      const ch = raw[i];
      if (ch === "(") {
        depth += 1;
        continue;
      }
      if (ch === ")") {
        depth -= 1;
        if (depth === 0) {
          return { args: raw.slice(start + 1, i), end: i };
        }
      }
    }
    return null;
  }
  function parseProcedural(raw) {
    const first = findNextProceduralOp(raw, 0);
    if (!first) {
      return null;
    }
    const base = raw.slice(0, first.index).trim();
    const ops = [];
    let cursor = first.index;
    while (cursor < raw.length) {
      const next = findNextProceduralOp(raw, cursor);
      if (!next) {
        break;
      }
      const parenStart = next.index + next.token.length - 1;
      const parsed = readParenContent(raw, parenStart);
      if (!parsed) {
        break;
      }
      ops.push({ type: next.type, args: parsed.args.trim() });
      cursor = parsed.end + 1;
    }
    if (ops.length === 0) {
      return null;
    }
    return { base: base || "*", ops };
  }
  function stripQuotes(value) {
    const trimmed = value.trim();
    if (trimmed.startsWith('"') && trimmed.endsWith('"') || trimmed.startsWith("'") && trimmed.endsWith("'")) {
      return trimmed.slice(1, -1);
    }
    return trimmed;
  }
  function applyProceduralRules(rules) {
    const limit = Math.min(rules.length, MAX_PROCEDURAL_RULES);
    for (let i = 0;i < limit; i++) {
      const raw = rules[i];
      if (!raw) {
        continue;
      }
      const parsed = parseProcedural(raw);
      if (!parsed) {
        continue;
      }
      let nodes = [];
      try {
        nodes = Array.from(document.querySelectorAll(parsed.base));
      } catch (e) {
        continue;
      }
      if (nodes.length > MAX_PROCEDURAL_NODES) {
        nodes = nodes.slice(0, MAX_PROCEDURAL_NODES);
      }
      for (const op of parsed.ops) {
        if (nodes.length === 0) {
          break;
        }
        if (op.type === "has-text") {
          const needle = stripQuotes(op.args).toLowerCase();
          if (!needle) {
            continue;
          }
          nodes = nodes.filter((node) => (node.textContent || "").toLowerCase().includes(needle));
          continue;
        }
        if (op.type === "matches-css") {
          const parts = op.args.split(",");
          const prop = parts[0] ? parts[0].trim() : "";
          const value = parts[1] ? stripQuotes(parts.slice(1).join(",")).trim() : "";
          if (!prop || !value) {
            continue;
          }
          nodes = nodes.filter((node) => {
            const style = window.getComputedStyle(node);
            const current = style.getPropertyValue(prop).trim();
            return current === value || current.includes(value);
          });
          continue;
        }
        if (op.type === "xpath") {
          const expr = stripQuotes(op.args);
          if (!expr) {
            continue;
          }
          const nextNodes = [];
          const seen = new Set;
          for (const node of nodes) {
            let snapshot = null;
            try {
              snapshot = document.evaluate(expr, node, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
            } catch (e) {
              continue;
            }
            if (!snapshot) {
              continue;
            }
            const length = Math.min(snapshot.snapshotLength, MAX_PROCEDURAL_NODES);
            for (let idx = 0;idx < length; idx++) {
              const item = snapshot.snapshotItem(idx);
              if (item && item.nodeType === Node.ELEMENT_NODE) {
                const element = item;
                if (!seen.has(element)) {
                  seen.add(element);
                  nextNodes.push(element);
                }
              }
            }
            if (nextNodes.length >= MAX_PROCEDURAL_NODES) {
              break;
            }
          }
          nodes = nextNodes;
          continue;
        }
        if (op.type === "upward") {
          const rawArg = stripQuotes(op.args);
          if (!rawArg) {
            continue;
          }
          const asNumber = Number(rawArg);
          const nextNodes = [];
          const seen = new Set;
          for (const node of nodes) {
            let target = null;
            if (Number.isFinite(asNumber)) {
              let current = node;
              let steps = Math.max(0, Math.floor(asNumber));
              while (current && steps > 0) {
                current = current.parentElement;
                steps -= 1;
              }
              target = current;
            } else {
              target = node.closest(rawArg);
            }
            if (target && !seen.has(target)) {
              seen.add(target);
              nextNodes.push(target);
            }
            if (nextNodes.length >= MAX_PROCEDURAL_NODES) {
              break;
            }
          }
          nodes = nextNodes;
          continue;
        }
        if (op.type === "style") {
          const styleText = op.args.trim();
          if (!styleText) {
            continue;
          }
          for (const node of nodes) {
            if (node instanceof HTMLElement) {
              const existing = node.getAttribute("style");
              const next = existing ? `${existing};${styleText}` : styleText;
              node.setAttribute("style", next);
            }
          }
          continue;
        }
        if (op.type === "remove") {
          for (const node of nodes) {
            node.remove();
          }
          nodes = [];
        }
      }
    }
  }
  (async () => {
    if (document.documentElement.dataset.bbInjected) {
      return;
    }
    document.documentElement.dataset.bbInjected = "1";
    const global = window;
    if (!global.__bbScriptlets) {
      global.__bbScriptlets = scriptlets;
    }
    try {
      const response = await sendMessage({
        type: "cosmetic.get",
        url: window.location.href
      });
      if (!response) {
        return;
      }
      if (response.css && response.css.length > 0) {
        const style = document.createElement("style");
        style.id = "bb-injected-style";
        style.textContent = response.css;
        (document.head || document.documentElement).appendChild(style);
      }
      if (response.procedural && response.procedural.length > 0) {
        applyProceduralRules(response.procedural);
      }
      if (response.scriptlets && response.scriptlets.length > 0) {
        const pageCalls = [];
        const localCalls = [];
        const limit = Math.min(response.scriptlets.length, MAX_SCRIPTLETS);
        for (let i = 0;i < limit; i++) {
          const call = response.scriptlets[i];
          if (!call) {
            continue;
          }
          if (pageContextScriptlets.has(call.name)) {
            pageCalls.push(call);
          } else {
            localCalls.push(call);
          }
        }
        if (localCalls.length > 0) {
          for (const call of localCalls) {
            const fn = scriptlets[call.name];
            if (!fn) {
              continue;
            }
            const args = Array.isArray(call.args) ? call.args.slice(0, MAX_SCRIPTLET_ARGS) : [];
            try {
              fn(args);
            } catch (e) {}
          }
        }
        if (pageCalls.length > 0 && isTopFrame()) {
          injectPageScriptlets(pageCalls);
        }
      }
    } catch (e) {}
  })();
})();
