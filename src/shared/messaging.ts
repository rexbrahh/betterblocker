/// <reference types="chrome"/>

export type MessageBase = { type: string; payload?: unknown } & Record<string, unknown>;

export type SendMessageOptions = {
  retries?: number;
  retryDelayMs?: number;
};

export function sendMessage<T = unknown, M extends MessageBase = MessageBase>(
  message: M,
  options: SendMessageOptions = {}
): Promise<T> {
  const { retries = 0, retryDelayMs = 250 } = options;
  const attempt = (remaining: number): Promise<T> =>
    new Promise((resolve) => {
      chrome.runtime.sendMessage(message, (response) => {
        if (chrome.runtime.lastError) {
          const errorMessage = chrome.runtime.lastError.message ?? String(chrome.runtime.lastError);
          console.warn('Message error:', message.type, errorMessage);
          if (remaining > 0) {
            setTimeout(() => {
              attempt(remaining - 1).then(resolve);
            }, retryDelayMs);
            return;
          }
          resolve({} as T);
          return;
        }
        resolve(response);
      });
    });

  return attempt(retries);
}

export function sendMessageStrict<T = unknown, M extends MessageBase = MessageBase>(message: M): Promise<T> {
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
