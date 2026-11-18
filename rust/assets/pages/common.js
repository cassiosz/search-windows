// The way to the app: a question posted, its answer matched by id. The app
// answers only pages whose address is search://, so a web page asking gets nothing.
const pt = navigator.language.toLowerCase().startsWith('pt');
const waiting = new Map();
let asked = 0;
function ask(op, data) {
  return new Promise(done => {
    const id = ++asked;
    waiting.set(id, done);
    window.chrome.webview.postMessage(Object.assign({}, data, {op, id}));
  });
}
window.chrome.webview.addEventListener('message', e => {
  const done = waiting.get(e.data && e.data.id);
  if (done) { waiting.delete(e.data.id); done(e.data); }
  // News the app sends unasked (a download's progress), for the page that listens.
  else if (e.data && e.data.event && window.onAppEvent) window.onAppEvent(e.data);
});
const $ = id => document.getElementById(id);
function make(tag, props, kids) {
  const el = Object.assign(document.createElement(tag), props);
  for (const kid of kids || []) el.append(kid);
  return el;
}
// Ctrl+F finds in the page's own list, as upstream's ⌘F does in Settings.
addEventListener('keydown', e => {
  if (e.ctrlKey && !e.altKey && e.key.toLowerCase() === 'f') { e.preventDefault(); const f = document.querySelector('input[type=search]'); f.focus(); f.select(); }
});
