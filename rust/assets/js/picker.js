// Ported from Search (Office Commun), MIT (Curtain.swift, Veiling.picker)
// The pointing mode for "hide anything": hover outline, click picks, Esc cancels.
// Self-contained and idempotent. Asleep until started: it costs one closure.
(function () {
  if (window.__officeVeil) return;
  var frame = null, tag = null, target = null, live = false;
  var bridge = window.chrome && window.chrome.webview;

  function post(msg) {
    try { if (bridge) bridge.postMessage(msg); } catch (e) {}
  }

  function sheet(id) {
    var s = document.getElementById(id);
    if (!s) {
      s = document.createElement('style');
      s.id = id;
      (document.head || document.documentElement).appendChild(s);
    }
    return s;
  }

  // What is hidden lives in a constructed stylesheet, the one the app's
  // document-start script made (window.__officeVeilSheet), or one made now.
  // Rules go in one at a time: one the engine refuses takes no other with it.
  function veil(css) {
    var s = window.__officeVeilSheet;
    if (!(s instanceof CSSStyleSheet)) {
      s = new CSSStyleSheet();
      window.__officeVeilSheet = s;
    }
    if (document.adoptedStyleSheets.indexOf(s) < 0) {
      document.adoptedStyleSheets = document.adoptedStyleSheets.concat([s]);
    }
    while (s.cssRules.length) s.deleteRule(0);
    String(css || '').split('\n').forEach(function (rule) {
      if (rule) try { s.insertRule(rule, s.cssRules.length); } catch (e) {}
    });
  }

  function chrome() {
    if (frame) return frame;
    frame = document.createElement('div');
    frame.id = 'office-veil-frame';
    frame.style.cssText = 'position:fixed;z-index:2147483646;pointer-events:none;' +
      'border:2px solid rgba(23,23,23,.9);background:rgba(23,23,23,.07);' +
      'border-radius:4px;transition:all .07s ease-out;display:none';
    tag = document.createElement('div');
    tag.style.cssText = 'position:absolute;font:500 11px "Segoe UI",system-ui,' +
      'sans-serif;color:#fff;background:#171717;padding:2px 7px;' +
      'border-radius:5px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis';
    frame.appendChild(tag);
    document.documentElement.appendChild(frame);
    return frame;
  }

  function place(el) {
    var box = chrome(), r = el.getBoundingClientRect();
    box.style.display = 'block';
    box.style.left = r.left + 'px';
    box.style.top = r.top + 'px';
    box.style.width = r.width + 'px';
    box.style.height = r.height + 'px';
    tag.textContent = name(el);
    // Above the element if there is sky above it, tucked inside its top edge if not.
    tag.style.top = r.top >= 26 ? '-21px' : '3px';
    tag.style.left = Math.max(2, -r.left + 4) + 'px';
    tag.style.maxWidth = Math.max(80, window.innerWidth - Math.max(0, r.left) - 16) + 'px';
  }

  var known = {
    nav: 'Navigation', header: 'Header', footer: 'Footer', aside: 'Sidebar',
    form: 'Form', dialog: 'Dialog', video: 'Video', img: 'Image',
    button: 'Button', iframe: 'Embed', figure: 'Figure', table: 'Table'
  };

  function clip(text, n) { return text.length > n ? text.slice(0, n) + '…' : text; }

  // What it is: what it calls itself, then what kind of thing, then what it says.
  function name(el) {
    var said = el.getAttribute && (el.getAttribute('aria-label') || el.getAttribute('title'));
    if (said && said.trim()) return clip(said.trim(), 40);
    var tagName = el.tagName.toLowerCase();
    if (known[tagName]) return known[tagName];
    var role = el.getAttribute && el.getAttribute('role');
    if (role) return role.charAt(0).toUpperCase() + role.slice(1);
    var text = (el.innerText || '').trim().replace(/\s+/g, ' ');
    return text ? clip(text, 40) : tagName;
  }

  // How big, and which corner: two sidebars read alike, rarely the same shape in the same place.
  function shape(el) {
    var r = el.getBoundingClientRect();
    var cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    var side = cx < window.innerWidth / 3 ? 'left'
             : (cx > window.innerWidth * 2 / 3 ? 'right' : 'centre');
    var band = cy < window.innerHeight / 3 ? 'top'
             : (cy > window.innerHeight * 2 / 3 ? 'bottom' : 'middle');
    return Math.round(r.width) + '×' + Math.round(r.height) + ' · ' + band + ' ' + side;
  }

  // A class worth hanging a rule on: a word, not a build artefact.
  function steady(c) {
    return /^[a-zA-Z][\w-]{2,29}$/.test(c) && !/\d{3,}/.test(c) &&
           !/^(css|sc|jsx|emotion|svelte|styles?)-/.test(c);
  }

  function unique(sel) {
    try { return document.querySelectorAll(sel).length === 1; } catch (e) { return false; }
  }

  function selectorFor(el) {
    if (el.id && unique('#' + CSS.escape(el.id))) return '#' + CSS.escape(el.id);

    var hooks = ['data-testid', 'data-test', 'data-qa', 'data-cy', 'aria-label', 'name', 'role'];
    for (var i = 0; i < hooks.length; i++) {
      var v = el.getAttribute && el.getAttribute(hooks[i]);
      if (v) {
        // Quoted attribute value: escape the quote and the backslash (CSS.escape is
        // for identifiers and would double up on spaces and punctuation here).
        var s = el.tagName.toLowerCase() + '[' + hooks[i] + '="' +
                v.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\a ') + '"]';
        if (unique(s)) return s;
      }
    }

    var classes = (el.className && typeof el.className === 'string')
      ? el.className.trim().split(/\s+/).filter(steady) : [];
    if (classes.length) {
      var byClass = el.tagName.toLowerCase() + '.' + classes.map(CSS.escape).join('.');
      if (unique(byClass)) return byClass;
    }

    // Last resort: a path, anchored on the nearest thing with a name.
    var parts = [], node = el;
    while (node && node.nodeType === 1 && node !== document.documentElement) {
      if (node.id && unique('#' + CSS.escape(node.id))) {
        parts.unshift('#' + CSS.escape(node.id));
        break;
      }
      var tagName = node.tagName.toLowerCase();
      var parent = node.parentElement;
      if (!parent) { parts.unshift(tagName); break; }
      var kin = Array.prototype.filter.call(parent.children, function (c) {
        return c.tagName === node.tagName;
      });
      parts.unshift(kin.length > 1
        ? tagName + ':nth-of-type(' + (kin.indexOf(node) + 1) + ')'
        : tagName);
      node = parent;
    }
    return parts.join(' > ');
  }

  function usable(el) {
    return el && el !== frame && el !== document.documentElement && el !== document.body &&
           !(frame && frame.contains(el));
  }

  function onMove(e) {
    if (!live) return;
    var el = document.elementFromPoint(e.clientX, e.clientY);
    if (!usable(el)) return;
    target = el;
    place(el);
  }

  // Everything a press can be, swallowed. Real pages act on pointerdown or
  // mousedown and are gone before a click ever completes.
  function swallow(e) {
    if (!live) return;
    e.preventDefault();
    e.stopPropagation();
    e.stopImmediatePropagation();
  }

  function onPress(e) {
    if (!live) return;
    swallow(e);
    var el = target || document.elementFromPoint(e.clientX, e.clientY);
    if (!usable(el)) return;
    try {
      post({
        op: 'curtain.pick',
        host: location.hostname.toLowerCase().replace(/^www\./, ''),
        selector: selectorFor(el),
        label: name(el),
        note: shape(el)
      });
    } catch (err) {
      post({ op: 'curtain.trouble', trouble: String(err) });
    }
    target = null;
    if (frame) frame.style.display = 'none';
  }

  function onKey(e) {
    if (!live) return;
    if (e.key === 'Escape') {
      swallow(e);
      api.off();
    }
  }

  var presses = ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click',
                 'dblclick', 'contextmenu', 'touchstart'];

  var api = window.__officeVeil = {
    on: function () {
      if (live) return;
      live = true;
      chrome();
      document.documentElement.style.cursor = 'crosshair';
      document.addEventListener('mousemove', onMove, true);
      document.addEventListener('pointermove', onMove, true);
      document.addEventListener('keydown', onKey, true);
      presses.forEach(function (kind) {
        document.addEventListener(kind, kind === 'pointerdown' ? onPress : swallow, true);
      });
    },
    off: function () {
      if (!live) return;
      live = false;
      target = null;
      if (frame) frame.style.display = 'none';
      document.documentElement.style.cursor = '';
      document.removeEventListener('mousemove', onMove, true);
      document.removeEventListener('pointermove', onMove, true);
      document.removeEventListener('keydown', onKey, true);
      presses.forEach(function (kind) {
        document.removeEventListener(kind, kind === 'pointerdown' ? onPress : swallow, true);
      });
      post({ op: 'curtain.off' });
    },
    // Replace the hiding stylesheet (the same sheet the document-start script made),
    // so a pick takes effect at once.
    css: function (css) {
      veil(css);
    },
    // Show one hidden thing, outlined, for as long as the pointer rests on its row.
    peek: function (css, sel) {
      veil(css);
      sheet('office-peek').textContent = sel +
        ' { outline: 2px solid rgba(23,23,23,.9) !important; outline-offset: 2px !important; }';
      try {
        var el = document.querySelector(sel);
        if (el) el.scrollIntoView({ block: 'center', behavior: 'smooth' });
      } catch (e) {}
    },
    unpeek: function (css) {
      veil(css);
      sheet('office-peek').textContent = '';
    }
  };

  // The same controls by message, for an app that prefers PostWebMessageAsJson:
  // {op:'curtain.start'} {op:'curtain.stop'} {op:'curtain.css', css}
  if (bridge && bridge.addEventListener) {
    bridge.addEventListener('message', function (e) {
      var m = e && e.data;
      if (!m || typeof m !== 'object') return;
      if (m.op === 'curtain.start') api.on();
      else if (m.op === 'curtain.stop') api.off();
      else if (m.op === 'curtain.css') api.css(m.css);
    });
  }
})();
