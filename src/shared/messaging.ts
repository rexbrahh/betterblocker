/// <reference types="chrome"/>

export function sendMessage<T>(message: { type: string; payload?: unknown }): Promise<T> {
  return new Promise((resolve) => {
    chrome.runtime.sendMessage(message, (response) => {
      if (chrome.runtime.lastError) {
        console.warn('Message error:', chrome.runtime.lastError);
        resolve({} as T);
      } else {
        resolve(response);
      }
    });
  });
}

export function sendMessageStrict<T>(message: { type: string; payload?: unknown }): Promise<T> {
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
