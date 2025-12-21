export interface ListStats {
  lines: number;
  rulesBefore: number;
  rulesAfter: number;
}

export interface SnapshotStats {
  rulesBefore: number;
  rulesAfter: number;
  listStats: ListStats[];
}

export interface StoredSnapshot {
  data: Uint8Array;
  stats: SnapshotStats;
  updatedAt: string;
  sourceUrls: string[];
}

const DB_NAME = 'betterblocker';
const DB_VERSION = 1;
const STORE_NAME = 'snapshots';
const ACTIVE_KEY = 'active';

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);

    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME, { keyPath: 'key' });
      }
    };

    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error('Failed to open IndexedDB'));
  });
}

export async function loadStoredSnapshot(): Promise<StoredSnapshot | null> {
  const db = await openDb();

  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readonly');
    const store = tx.objectStore(STORE_NAME);
    const request = store.get(ACTIVE_KEY);

    request.onsuccess = () => {
      resolve((request.result as StoredSnapshot | undefined) ?? null);
    };
    request.onerror = () => {
      reject(request.error ?? new Error('Failed to read snapshot'));
    };

    tx.oncomplete = () => db.close();
    tx.onerror = () => {
      db.close();
    };
  });
}

export async function saveStoredSnapshot(record: StoredSnapshot): Promise<void> {
  const db = await openDb();

  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readwrite');
    const store = tx.objectStore(STORE_NAME);

    store.put({ key: ACTIVE_KEY, ...record });

    tx.oncomplete = () => {
      db.close();
      resolve();
    };
    tx.onerror = () => {
      db.close();
      reject(tx.error ?? new Error('Failed to save snapshot'));
    };
  });
}

export async function clearStoredSnapshot(): Promise<void> {
  const db = await openDb();

  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, 'readwrite');
    const store = tx.objectStore(STORE_NAME);

    store.delete(ACTIVE_KEY);

    tx.oncomplete = () => {
      db.close();
      resolve();
    };
    tx.onerror = () => {
      db.close();
      reject(tx.error ?? new Error('Failed to clear snapshot'));
    };
  });
}
