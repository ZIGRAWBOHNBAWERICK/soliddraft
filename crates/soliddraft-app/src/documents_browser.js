// The browser build's documents: recovery copies and the recent-documents list, kept in
// this site's IndexedDB. That storage belongs to one browser profile; nothing here is
// sent to the server.
//
// Recovery copies are keyed "<session>/<file>". Each tab holds a Web Lock on its own
// session for as long as it is open, so a session whose lock is free belonged to a tab
// that was closed or crashed. At start, the copies of such sessions move under this
// session's "offered/" prefix, while their lock is held, so two tabs never adopt the
// same copy. An offer left undecided stays there, and the next visit finds it again.
const NAME = "soliddraft-documents";
const RECOVERY = "recovery";
const RECENT = "recent";
const LOCK = "soliddraft-recovery/";
const RECENT_LIMIT = 8;

let session = null;
let database = null;

function request(r) {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

function done(tx) {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error || new Error("the browser stopped the write"));
  });
}

function db() {
  if (!database) {
    database = new Promise((resolve, reject) => {
      if (!globalThis.indexedDB) {
        reject(new Error("this browser does not let the page keep documents"));
        return;
      }
      const open = indexedDB.open(NAME, 1);
      open.onupgradeneeded = () => {
        const d = open.result;
        if (!d.objectStoreNames.contains(RECOVERY)) d.createObjectStore(RECOVERY);
        if (!d.objectStoreNames.contains(RECENT)) d.createObjectStore(RECENT);
      };
      open.onsuccess = () => resolve(open.result);
      open.onerror = () => reject(open.error);
      open.onblocked = () => reject(new Error("another SolidDraft tab is updating the browser's storage"));
    });
  }
  return database;
}

function range(prefix) {
  return IDBKeyRange.bound(prefix, prefix + "￿");
}

function message(error) {
  if (error && error.name === "QuotaExceededError") return "the browser's storage for this site is full";
  return String((error && error.message) || error);
}

async function guarded(work) {
  try {
    return await work();
  } catch (error) {
    throw new Error(message(error));
  }
}

// Moves every copy of the ended session `other` under this session's offered prefix.
async function adoptSession(d, other) {
  const tx = d.transaction(RECOVERY, "readwrite");
  const store = tx.objectStore(RECOVERY);
  const keys = await request(store.getAllKeys(range(other + "/")));
  for (const key of keys) {
    const value = await request(store.get(key));
    let to = `${session}/offered/${key.split("/").pop()}`;
    let there = await request(store.get(to));
    const same = there && there.bytes.length === value.bytes.length &&
      there.bytes.every((byte, i) => byte === value.bytes[i]);
    if (!same) {
      // Different sessions can hold independent edits of the same document.
      while (there) {
        to = `${session}/offered/${crypto.randomUUID()}-${key.split("/").pop()}`;
        there = await request(store.get(to));
      }
      store.put(value, to);
    }
    store.delete(key);
  }
  await done(tx);
}

// Claims session `id`, adopts the copies of sessions that ended, and returns the
// offers: [{key, bytes}].
export async function start(id) {
  return guarded(async () => {
    if (!navigator.locks) throw new Error("recovery needs Web Locks on a secure origin");
    session = id;
    const d = await db();
    await new Promise(acquired => {
      navigator.locks.request(LOCK + id, () => {
        acquired();
        return new Promise(() => {});
      });
    });
    const keys = await request(d.transaction(RECOVERY).objectStore(RECOVERY).getAllKeys());
    const others = [...new Set(keys.map(k => k.split("/")[0]))].filter(s => s !== id);
    for (const other of others) {
      await navigator.locks.request(LOCK + other, {ifAvailable: true}, async lock => {
        if (lock) await adoptSession(d, other);
      });
    }
    const store = d.transaction(RECOVERY).objectStore(RECOVERY);
    const offered = range(`${id}/offered/`);
    const [offerKeys, values] = await Promise.all([
      request(store.getAllKeys(offered)),
      request(store.getAll(offered)),
    ]);
    return offerKeys.map((key, i) => ({key, bytes: values[i].bytes}));
  });
}

// Keeps a copy of this session's document file `file`.
export async function put(file, bytes, written) {
  return guarded(async () => {
    const tx = (await db()).transaction(RECOVERY, "readwrite");
    tx.objectStore(RECOVERY).put({bytes, written}, `${session}/${file}`);
    await done(tx);
  });
}

export async function remove(file) {
  return discard(`${session}/${file}`);
}

// Makes the offer `key` this session's copy of `file`.
export async function adopt(key, file) {
  return guarded(async () => {
    const tx = (await db()).transaction(RECOVERY, "readwrite");
    const store = tx.objectStore(RECOVERY);
    const value = await request(store.get(key));
    if (value) store.put(value, `${session}/${file}`);
    store.delete(key);
    await done(tx);
  });
}

export async function discard(key) {
  return guarded(async () => {
    const tx = (await db()).transaction(RECOVERY, "readwrite");
    tx.objectStore(RECOVERY).delete(key);
    await done(tx);
  });
}

// The recent documents, newest first: [{key, name, kind, when, bytes}].
export async function recent() {
  return guarded(async () => {
    const all = await request((await db()).transaction(RECENT).objectStore(RECENT).getAll());
    return all.sort((a, b) => b.when - a.when).slice(0, RECENT_LIMIT);
  });
}

// Puts a document at the top of the recent list, under `key`, and drops the oldest past
// the limit.
export async function remember(key, name, kind, when, bytes) {
  return guarded(async () => {
    const tx = (await db()).transaction(RECENT, "readwrite");
    const store = tx.objectStore(RECENT);
    store.put({key, name, kind, when, bytes}, key);
    const all = await request(store.getAll());
    all.sort((a, b) => b.when - a.when);
    for (const old of all.slice(RECENT_LIMIT)) store.delete(old.key);
    await done(tx);
  });
}

// Gives the recent document `key` a picture, a PNG, when the file has none of its own.
export async function thumbnail(key, png) {
  return guarded(async () => {
    const tx = (await db()).transaction(RECENT, "readwrite");
    const store = tx.objectStore(RECENT);
    const entry = await request(store.get(key));
    if (entry) store.put({...entry, thumbnail: png}, key);
    await done(tx);
  });
}

export async function forget() {
  return guarded(async () => {
    const tx = (await db()).transaction(RECENT, "readwrite");
    tx.objectStore(RECENT).clear();
    await done(tx);
  });
}
