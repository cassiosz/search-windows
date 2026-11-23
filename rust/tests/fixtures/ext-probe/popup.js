// Everything the popup can find out, in window.probe for the bench to read.
const step = (name, run) => Promise.resolve().then(run).then(v => [name, v], e => [name, 'error: ' + e.message]);
Promise.all([
  step('id', () => chrome.runtime.id),
  step('sw', () => chrome.storage.local.get('sw').then(r => r.sw || null)),
  step('active', () => chrome.tabs.query({ active: true, currentWindow: true }).then(t => t.map(x => ({ id: x.id, url: x.url })))),
  step('tabs', () => chrome.tabs.query({}).then(t => t.length)),
  step('alarm', () => chrome.alarms.create('probe', { delayInMinutes: 1 }).then(() => 'ok')),
]).then(pairs => {
  window.probe = Object.fromEntries(pairs);
  document.getElementById('out').textContent = JSON.stringify(window.probe);
});
