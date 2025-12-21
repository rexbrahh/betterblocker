type ScriptletFn = (args: string[]) => void;

type AnyRecord = Record<string, unknown>;

const MAX_SELECTOR_MATCHES = 200;

function parseValue(raw?: string): unknown {
  if (raw === undefined) {
    return undefined;
  }
  const trimmed = raw.trim();
  if (trimmed === 'null') {
    return null;
  }
  if (trimmed === 'true') {
    return true;
  }
  if (trimmed === 'false') {
    return false;
  }
  if (trimmed === 'undefined') {
    return undefined;
  }
  if (trimmed === '') {
    return '';
  }
  const numeric = Number(trimmed);
  if (!Number.isNaN(numeric) && String(numeric) === trimmed) {
    return numeric;
  }
  return raw;
}

function setConstant(args: string[]): void {
  const path = args[0];
  if (!path) {
    return;
  }
  const value = parseValue(args[1]);
  const parts = path.split('.').filter((part) => part.length > 0);
  if (parts.length === 0) {
    return;
  }
  let target: AnyRecord = window as unknown as AnyRecord;
  for (let i = 0; i < parts.length - 1; i++) {
    const key = parts[i];
    if (!key) {
      return;
    }
    const next = target[key];
    if (typeof next !== 'object' || next === null) {
      return;
    }
    target = next as AnyRecord;
  }
  const prop = parts[parts.length - 1];
  if (!prop) {
    return;
  }
  try {
    Object.defineProperty(target, prop, {
      value,
      configurable: true,
      writable: false,
    });
  } catch (e) {
    try {
      target[prop] = value;
    } catch (err) {
      void err;
    }
    void e;
  }
}

function forEachNode(selector: string, fn: (node: Element) => void): void {
  if (!selector) {
    return;
  }
  let nodes: Element[] = [];
  try {
    nodes = Array.from(document.querySelectorAll(selector));
  } catch (e) {
    void e;
    return;
  }
  const limit = Math.min(nodes.length, MAX_SELECTOR_MATCHES);
  for (let i = 0; i < limit; i++) {
    const node = nodes[i];
    if (!node) {
      continue;
    }
    fn(node);
  }
}

function removeAttr(args: string[]): void {
  const selector = args[0];
  const attr = args[1];
  if (!selector || !attr) {
    return;
  }
  forEachNode(selector, (node) => node.removeAttribute(attr));
}

function removeClass(args: string[]): void {
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

function addClass(args: string[]): void {
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

function hideBySelector(args: string[]): void {
  const selector = args[0];
  if (!selector) {
    return;
  }
  forEachNode(selector, (node) => {
    if (node instanceof HTMLElement) {
      node.style.setProperty('display', 'none', 'important');
    }
  });
}

function removeBySelector(args: string[]): void {
  const selector = args[0];
  if (!selector) {
    return;
  }
  forEachNode(selector, (node) => {
    node.remove();
  });
}

export const scriptlets: Record<string, ScriptletFn> = {
  'set-constant': setConstant,
  'remove-attr': removeAttr,
  'remove-class': removeClass,
  'add-class': addClass,
  'hide-by-selector': hideBySelector,
  'remove-by-selector': removeBySelector,
};
