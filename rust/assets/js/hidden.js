// Ported from Search (Office Commun), MIT (Hidden.swift: the "what is hidden here" panel,
// drawn in the page instead of natively). Needs picker.js (window.__officeVeil) for peeking.
// Resting on a row puts that one thing back, outlined, and scrolls to it; leaving puts the page back.
(function () {
  if (window.__officeHidden) return;
  var bridge = window.chrome && window.chrome.webview;
  var host = null, root = null, veils = [], hostname = '';

  function post(msg) {
    try { if (bridge) bridge.postMessage(msg); } catch (e) {}
  }

  function cssWithout(spared) {
    return veils.filter(function (v) { return v.selector !== spared; })
      .map(function (v) { return v.selector + ' { display: none !important; }'; }).join('\n');
  }

  function peek(v) {
    var p = window.__officeVeil;
    if (p && p.peek) p.peek(cssWithout(v.selector), v.selector);
  }

  function unpeek() {
    var p = window.__officeVeil;
    if (p && p.unpeek) p.unpeek(cssWithout(null));
  }

  var style =
    ':host{all:initial}' +
    '.p{position:fixed;top:16px;right:16px;width:380px;max-width:calc(100vw - 32px);z-index:2147483647;' +
    'font:13px/1.4 "Segoe UI",system-ui,sans-serif;color:#171717;background:#fff;border:1px solid #e8e8e8;' +
    'border-radius:12px;box-shadow:0 8px 30px rgba(0,0,0,.18);overflow:hidden}' +
    '@media (prefers-color-scheme:dark){.p{color:#e0e0e0;background:#1c1c1c;border-color:#333}' +
    '.row:hover{background:#2a2a2a}.q,.sub{color:#8a8a8a}.b{border-color:#444;color:#e0e0e0}}' +
    '.h{display:flex;align-items:center;padding:12px 14px;font-weight:600}' +
    '.h span{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}' +
    '.x{cursor:pointer;border:0;background:none;color:inherit;font:inherit;padding:0 4px}' +
    '.q{color:#a3a3a3;padding:0 14px 8px;font-size:11.5px}' +
    '.l{max-height:320px;overflow:auto}' +
    '.row{display:flex;align-items:center;gap:10px;padding:9px 14px}' +
    '.row:hover{background:#f5f5f5}.t{flex:1;min-width:0}' +
    '.t div{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}' +
    '.sub{color:#a3a3a3;font-size:11.5px}' +
    '.r{visibility:hidden}.row:hover .r{visibility:visible}' +
    '.b{cursor:pointer;border:1px solid #d4d4d4;background:none;color:inherit;font:inherit;' +
    'border-radius:999px;padding:3px 11px}' +
    '.f{display:flex;gap:8px;padding:10px 14px}';

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  function draw() {
    if (!root) return;
    root.textContent = '';
    var st = document.createElement('style');
    st.textContent = style;
    root.appendChild(st);
    var p = el('div', 'p');
    var h = el('div', 'h');
    h.appendChild(el('span', '', hostname || 'This page'));
    var x = el('button', 'x', '×');
    x.title = 'Close';
    x.onclick = close;
    h.appendChild(x);
    p.appendChild(h);
    if (!veils.length) {
      p.appendChild(el('div', 'q', 'Nothing is hidden here.'));
    } else {
      p.appendChild(el('div', 'q', 'Hidden on this site — rest on a line to see it'));
      var list = el('div', 'l');
      veils.forEach(function (v) {
        var row = el('div', 'row');
        var t = el('div', 't');
        t.appendChild(el('div', '', v.label || v.selector));
        if (v.note) t.appendChild(el('div', 'sub', v.note));
        row.appendChild(t);
        var r = el('button', 'b r', 'Restore');
        r.onclick = function () {
          unpeek();
          veils = veils.filter(function (o) { return o.selector !== v.selector; });
          post({ op: 'curtain.restore', host: hostname, selector: v.selector });
          draw();
        };
        row.appendChild(r);
        row.onmouseenter = function () { peek(v); };
        row.onmouseleave = unpeek;
        list.appendChild(row);
      });
      p.appendChild(list);
    }
    var f = el('div', 'f');
    var hide = el('button', 'b', 'Hide something…');
    hide.onclick = function () {
      close();
      post({ op: 'curtain.hideSomething', host: hostname });
    };
    f.appendChild(hide);
    if (veils.length) {
      var all = el('button', 'b', 'Restore all');
      all.onclick = function () {
        unpeek();
        veils = [];
        post({ op: 'curtain.restoreAll', host: hostname });
        draw();
      };
      f.appendChild(all);
    }
    p.appendChild(f);
    root.appendChild(p);
  }

  function onKey(e) {
    if (e.key === 'Escape') { e.stopPropagation(); close(); }
  }

  function close() {
    unpeek();
    document.removeEventListener('keydown', onKey, true);
    if (host) host.remove();
    host = root = null;
  }

  // list: [{selector, label, note}] as the app holds them for this host.
  function open(list) {
    veils = Array.isArray(list) ? list.filter(function (v) { return v && typeof v.selector === 'string'; }) : [];
    hostname = location.hostname.toLowerCase().replace(/^www\./, '');
    if (!host) {
      host = document.createElement('div');
      host.id = 'office-hidden-panel';
      root = host.attachShadow({ mode: 'closed' });
      document.documentElement.appendChild(host);
      document.addEventListener('keydown', onKey, true);
    }
    draw();
  }

  window.__officeHidden = {
    open: open, close: close,
    toggle: function (list) { if (host) close(); else open(list); }
  };

  // By message: {op:'curtain.list', veils:[...]} opens (or refreshes) the panel; {op:'curtain.closeList'} closes it.
  if (bridge && bridge.addEventListener) {
    bridge.addEventListener('message', function (e) {
      var m = e && e.data;
      if (!m || typeof m !== 'object') return;
      if (m.op === 'curtain.list') open(m.veils);
      else if (m.op === 'curtain.closeList') close();
    });
  }
})();
