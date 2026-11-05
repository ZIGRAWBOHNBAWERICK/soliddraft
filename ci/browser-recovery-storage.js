// Run through chrome-devtools-axi run on an HTTP server serving the repository root.
// Exercise the shipped storage module with real IndexedDB and Web Locks.
const result = await page.eval(async () => {
  const check = (value, message) => { if (!value) throw new Error(message); };
  await new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase('soliddraft-documents');
    request.onsuccess = resolve;
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('test database is in use'));
  });
  const database = await new Promise((resolve, reject) => {
    const request = indexedDB.open('soliddraft-documents', 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore('recovery');
      request.result.createObjectStore('recent');
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  const tx = database.transaction('recovery', 'readwrite');
  const store = tx.objectStore('recovery');
  store.put({bytes: new Uint8Array([1, 2, 3]), written: 1}, 'ended-one/doc.dwprt.recovery');
  store.put({bytes: new Uint8Array([4, 5, 6]), written: 2}, 'ended-two/doc.dwprt.recovery');
  store.put({bytes: new Uint8Array([1, 2, 3]), written: 3}, 'ended-duplicate/doc.dwprt.recovery');
  await new Promise((resolve, reject) => { tx.oncomplete = resolve; tx.onerror = () => reject(tx.error); });
  database.close();
  const storage = await import(`/crates/soliddraft-app/src/documents_browser.js?test=${Date.now()}`);
  const offers = await storage.start('test-session');
  check(offers.length === 2, 'adoption dropped divergent unsaved work');
  const bytes = offers.map(offer => Array.from(offer.bytes).join(',')).sort();
  check(JSON.stringify(bytes) === JSON.stringify(['1,2,3', '4,5,6']), 'adoption changed recovery bytes');
  return {offers: offers.length, bytes};
});
console.log(JSON.stringify(result));
const address = await page.eval(() => location.href);
await page.open(address);
const restarted = await page.eval(async () => {
  const storage = await import(`/crates/soliddraft-app/src/documents_browser.js?test=${Date.now()}`);
  const offers = await storage.start('next-session');
  if (offers.length !== 2) throw new Error('restart dropped a divergent recovery copy');
  const bytes = offers.map(offer => Array.from(offer.bytes).join(',')).sort();
  if (JSON.stringify(bytes) !== JSON.stringify(['1,2,3', '4,5,6'])) throw new Error('restart changed recovery bytes');
  await storage.discard(offers[0].key);
  await storage.adopt(offers[1].key, 'restored.dwprt.recovery');
  return {offers: offers.length, bytes};
});
console.log(JSON.stringify(restarted));
