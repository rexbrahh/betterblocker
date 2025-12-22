import { sendMessage } from '../shared/messaging.js';
import type { CosmeticPayload } from '../shared/types.js';
import { pageContextScriptlets, scriptlets } from './scriptlets.js';

const MAX_PROCEDURAL_NODES = 200;

function isTopFrame(): boolean {
  try {
    return window.top === window;
  } catch (e) {
    void e;
    return false;
  }
}

function injectPageScriptlets(calls: CosmeticPayload['scriptlets']): void {
  if (!calls || calls.length === 0) {
    return;
  }
  const limited = calls.map((call) => ({
    name: call.name,
    args: Array.isArray(call.args) ? call.args : [],
  }));
  const root = document.documentElement;
  if (!root) {
    return;
  }
  try {
    root.setAttribute('data-bb-scriptlets', JSON.stringify(limited));
  } catch (e) {
    void e;
  }

  const script = document.createElement('script');
  script.src = chrome.runtime.getURL('resources/scriptlets/registry.js');
  script.async = false;
  script.onload = () => script.remove();
  script.onerror = () => script.remove();
  (document.head || root).appendChild(script);

  try {
    document.dispatchEvent(new CustomEvent('bb-scriptlets', { detail: limited }));
  } catch (e) {
    void e;
  }
}

type ProceduralRule = CosmeticPayload['procedural'][number];

function applyProceduralRules(rules: ProceduralRule[]): void {
  for (const rule of rules) {
    if (!rule) {
      continue;
    }
    const base = rule.base?.trim() || '*';
    let nodes: Element[] = [];
    try {
      nodes = Array.from(document.querySelectorAll(base));
    } catch (e) {
      void e;
      continue;
    }
    if (nodes.length > MAX_PROCEDURAL_NODES) {
      nodes = nodes.slice(0, MAX_PROCEDURAL_NODES);
    }

    for (const op of rule.ops || []) {
      if (nodes.length === 0) {
        break;
      }
      if (op.type === 'has-text') {
        const needle = stripQuotes(op.args).toLowerCase();
        if (!needle) {
          continue;
        }
        nodes = nodes.filter((node) => (node.textContent || '').toLowerCase().includes(needle));
        continue;
      }
      if (op.type === 'matches-css') {
        const parts = op.args.split(',');
        const prop = parts[0] ? parts[0].trim() : '';
        const value = parts[1] ? stripQuotes(parts.slice(1).join(',')).trim() : '';
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
      if (op.type === 'xpath') {
        const expr = stripQuotes(op.args);
        if (!expr) {
          continue;
        }
        const nextNodes: Element[] = [];
        const seen = new Set<Element>();
        for (const node of nodes) {
          let snapshot: XPathResult | null = null;
          try {
            snapshot = document.evaluate(
              expr,
              node,
              null,
              XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,
              null
            );
          } catch (e) {
            void e;
            continue;
          }
          if (!snapshot) {
            continue;
          }
          const length = Math.min(snapshot.snapshotLength, MAX_PROCEDURAL_NODES);
          for (let idx = 0; idx < length; idx++) {
            const item = snapshot.snapshotItem(idx);
            if (item && item.nodeType === Node.ELEMENT_NODE) {
              const element = item as Element;
              if (!seen.has(element)) {
                seen.add(element);
                nextNodes.push(element);
              }
            }
          }
        }
        nodes = nextNodes;
        continue;
      }
      if (op.type === 'upward') {
        const rawArg = stripQuotes(op.args);
        if (!rawArg) {
          continue;
        }
        const asNumber = Number(rawArg);
        const nextNodes: Element[] = [];
        const seen = new Set<Element>();
        for (const node of nodes) {
          let target: Element | null = null;
          if (Number.isFinite(asNumber)) {
            let current: Element | null = node;
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
      if (op.type === 'style') {
        const styleText = op.args.trim();
        if (!styleText) {
          continue;
        }
        for (const node of nodes) {
          if (node instanceof HTMLElement) {
            const existing = node.getAttribute('style');
            const next = existing ? `${existing};${styleText}` : styleText;
            node.setAttribute('style', next);
          }
        }
        continue;
      }
      if (op.type === 'remove') {
        for (const node of nodes) {
          node.remove();
        }
        nodes = [];
      }
    }
  }
}

function stripQuotes(value: string): string {
  const trimmed = value.trim();
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

(async () => {
  if (document.documentElement.dataset.bbInjected) {
    return;
  }
  document.documentElement.dataset.bbInjected = '1';

  const global = window as unknown as Record<string, unknown>;
  if (!global.__bbScriptlets) {
    global.__bbScriptlets = scriptlets;
  }

  try {
    const response = await sendMessage<CosmeticPayload>(
      {
        type: 'cosmetic.get',
        url: window.location.href,
      },
      { retries: 2, retryDelayMs: 250 }
    );

    if (!response) {
      return;
    }

    if (response.css && response.css.length > 0) {
      const style = document.createElement('style');
      style.id = 'bb-injected-style';
      style.textContent = response.css;
      (document.head || document.documentElement).appendChild(style);
    }

    if (response.procedural && response.procedural.length > 0) {
      applyProceduralRules(response.procedural);
    }

    if (response.scriptlets && response.scriptlets.length > 0) {
      const pageCalls: CosmeticPayload['scriptlets'] = [];
      const localCalls: CosmeticPayload['scriptlets'] = [];
      for (const call of response.scriptlets) {
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
          const args = Array.isArray(call.args) ? call.args : [];
          try {
            fn(args);
          } catch (e) {
            void e;
          }
        }
      }

      if (pageCalls.length > 0 && isTopFrame()) {
        injectPageScriptlets(pageCalls);
      }
    }
  } catch (e) {
    void e;
  }
})();
