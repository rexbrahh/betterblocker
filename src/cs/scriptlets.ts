type ScriptletFn = (args: unknown[]) => void;

type AnyRecord = Record<string, unknown>;

const MAX_SELECTOR_MATCHES = 200;

function readStringArg(args: unknown[], index: number): string {
  const value = args[index];
  return typeof value === 'string' ? value : '';
}

function setConstant(args: unknown[]): void {
  const path = readStringArg(args, 0);
  if (!path) {
    return;
  }
  const value = args.length > 1 ? args[1] : undefined;
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

function removeAttr(args: unknown[]): void {
  const selector = readStringArg(args, 0);
  const attr = readStringArg(args, 1);
  if (!selector || !attr) {
    return;
  }
  forEachNode(selector, (node) => node.removeAttribute(attr));
}

function removeClass(args: unknown[]): void {
  const selector = readStringArg(args, 0);
  const className = readStringArg(args, 1);
  if (!selector || !className) {
    return;
  }
  forEachNode(selector, (node) => {
    if (node instanceof HTMLElement) {
      node.classList.remove(className);
    }
  });
}

function addClass(args: unknown[]): void {
  const selector = readStringArg(args, 0);
  const className = readStringArg(args, 1);
  if (!selector || !className) {
    return;
  }
  forEachNode(selector, (node) => {
    if (node instanceof HTMLElement) {
      node.classList.add(className);
    }
  });
}

function hideBySelector(args: unknown[]): void {
  const selector = readStringArg(args, 0);
  if (!selector) {
    return;
  }
  forEachNode(selector, (node) => {
    if (node instanceof HTMLElement) {
      node.style.setProperty('display', 'none', 'important');
    }
  });
}

function removeBySelector(args: unknown[]): void {
  const selector = readStringArg(args, 0);
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

export const pageContextScriptlets = new Set(['set-constant']);
