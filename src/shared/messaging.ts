/// <reference types="chrome"/>

export type MessageBase = { type: string; payload?: unknown } & Record<string, unknown>;

export function sendMessage<T = unknown, M extends MessageBase = MessageBase>(message: M): Promise<T> {
  return new Promise((resolve) => {
    chrome.runtime.sendMessage(message, (response) => {
      if (chrome.runtime.lastError) {
        const errorMessage = chrome.runtime.lastError.message ?? String(chrome.runtime.lastError);
        console.warn('Message error:', message.type, errorMessage);
        resolve({} as T);
      } else {
        resolve(response);
      }
    });
  });
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
