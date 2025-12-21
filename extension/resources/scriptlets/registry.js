(function () {
  if (typeof window === 'undefined') {
    return;
  }
  var registry = window.__bbScriptlets;
  if (!registry || typeof registry !== 'object') {
    registry = {};
    try {
      Object.defineProperty(window, '__bbScriptlets', {
        value: registry,
        configurable: true,
      });
    } catch (e) {
      window.__bbScriptlets = registry;
      void e;
    }
  }

  var MAX_SELECTOR_MATCHES = 200;
  var MAX_SCRIPTLET_CALLS = 32;
  var MAX_SCRIPTLET_ARGS = 8;

  function runCalls(calls) {
    if (!Array.isArray(calls)) {
      return;
    }
    var limit = Math.min(calls.length, MAX_SCRIPTLET_CALLS);
    for (var i = 0; i < limit; i++) {
      var call = calls[i];
      if (!call || typeof call.name !== 'string') {
        continue;
      }
      var fn = registry[call.name];
      if (typeof fn !== 'function') {
        continue;
      }
      var args = Array.isArray(call.args) ? call.args.slice(0, MAX_SCRIPTLET_ARGS) : [];
      try {
        fn(args);
      } catch (e) {
        void e;
      }
    }
  }

  function runQueuedCalls() {
    var root = document.documentElement;
    if (!root) {
      return;
    }
    var raw = root.getAttribute('data-bb-scriptlets');
    if (!raw) {
      return;
    }
    root.removeAttribute('data-bb-scriptlets');
    try {
      var calls = JSON.parse(raw);
      runCalls(calls);
    } catch (e) {
      void e;
    }
  }

  function onScriptletEvent(event) {
    var detail = event && event.detail;
    runCalls(detail);
  }

  if (document && document.addEventListener) {
    document.addEventListener('bb-scriptlets', onScriptletEvent);
  }

  function parseValue(raw) {
    if (raw === undefined) {
      return undefined;
    }
    var trimmed = String(raw).trim();
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
    var numeric = Number(trimmed);
    if (!Number.isNaN(numeric) && String(numeric) === trimmed) {
      return numeric;
    }
    return raw;
  }

  function setConstant(args) {
    var path = args && args[0];
    if (!path) {
      return;
    }
    var value = parseValue(args[1]);
    var parts = String(path).split('.').filter(function (part) {
      return part.length > 0;
    });
    if (!parts.length) {
      return;
    }
    var target = window;
    for (var i = 0; i < parts.length - 1; i++) {
      var key = parts[i];
      if (!key) {
        return;
      }
      var next = target[key];
      if (typeof next !== 'object' || next === null) {
        return;
      }
      target = next;
    }
    var prop = parts[parts.length - 1];
    if (!prop) {
      return;
    }
    try {
      Object.defineProperty(target, prop, {
        value: value,
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

  function forEachNode(selector, fn) {
    if (!selector) {
      return;
    }
    var nodes = [];
    try {
      nodes = Array.prototype.slice.call(document.querySelectorAll(selector));
    } catch (e) {
      void e;
      return;
    }
    var limit = Math.min(nodes.length, MAX_SELECTOR_MATCHES);
    for (var i = 0; i < limit; i++) {
      var node = nodes[i];
      if (!node) {
        continue;
      }
      fn(node);
    }
  }

  function removeAttr(args) {
    var selector = args && args[0];
    var attr = args && args[1];
    if (!selector || !attr) {
      return;
    }
    forEachNode(selector, function (node) {
      node.removeAttribute(attr);
    });
  }

  function removeClass(args) {
    var selector = args && args[0];
    var className = args && args[1];
    if (!selector || !className) {
      return;
    }
    forEachNode(selector, function (node) {
      if (node.classList) {
        node.classList.remove(className);
      }
    });
  }

  function addClass(args) {
    var selector = args && args[0];
    var className = args && args[1];
    if (!selector || !className) {
      return;
    }
    forEachNode(selector, function (node) {
      if (node.classList) {
        node.classList.add(className);
      }
    });
  }

  function hideBySelector(args) {
    var selector = args && args[0];
    if (!selector) {
      return;
    }
    forEachNode(selector, function (node) {
      if (node.style && node.style.setProperty) {
        node.style.setProperty('display', 'none', 'important');
      }
    });
  }

  function removeBySelector(args) {
    var selector = args && args[0];
    if (!selector) {
      return;
    }
    forEachNode(selector, function (node) {
      if (node.remove) {
        node.remove();
      }
    });
  }

  registry['set-constant'] = setConstant;
  registry['remove-attr'] = removeAttr;
  registry['remove-class'] = removeClass;
  registry['add-class'] = addClass;
  registry['hide-by-selector'] = hideBySelector;
  registry['remove-by-selector'] = removeBySelector;

  runQueuedCalls();
})();
