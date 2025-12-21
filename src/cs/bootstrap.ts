import { sendMessage } from '../shared/messaging.js';
import type { CosmeticPayload } from '../shared/types.js';
import { scriptlets } from './scriptlets.js';

const MAX_SCRIPTLETS = 32;
const MAX_SCRIPTLET_ARGS = 8;

function isTopFrame(): boolean {
  try {
    return window.top === window;
  } catch (e) {
    void e;
    return false;
  }
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
    const response = await sendMessage<CosmeticPayload>({
      type: 'cosmetic.get',
      url: window.location.href,
    });

    if (!response) {
      return;
    }

    if (response.css && response.css.length > 0) {
      const style = document.createElement('style');
      style.id = 'bb-injected-style';
      style.textContent = response.css;
      (document.head || document.documentElement).appendChild(style);
    }

    if (response.scriptlets && response.scriptlets.length > 0 && isTopFrame()) {
      const limit = Math.min(response.scriptlets.length, MAX_SCRIPTLETS);
      for (let i = 0; i < limit; i++) {
        const call = response.scriptlets[i];
        if (!call) {
          continue;
        }
        const fn = scriptlets[call.name];
        if (!fn) {
          continue;
        }
        const args = Array.isArray(call.args) ? call.args.slice(0, MAX_SCRIPTLET_ARGS) : [];
        try {
          fn(args);
        } catch (e) {
          void e;
        }
      }
    }
  } catch (e) {
    void e;
  }
})();
