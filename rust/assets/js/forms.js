// Sign-in boxes (upstream Forms.swift, its rules cut to what the offers
// need): which box the caret is in, where it sits, when a sign-in goes out,
// and when its boxes go away in place. Never a password in a message: the
// app asks for the one just sent (take), once, from this document only.
(function () {
  if (window.__officeForms || !/^https?:$/.test(location.protocol) || !window.chrome || !chrome.webview) return;
  // Taken now, before the page's own scripts can put something in their place.
  var send = chrome.webview.postMessage.bind(chrome.webview);
  var setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
  function post(m) { try { send(m); } catch (e) {} }
  function seen(b) { var r = b.getBoundingClientRect(); return r.width > 0 && r.height > 0; }
  function kind(el) { return (el.type || 'text').toLowerCase(); }
  function isInput(el) { return !!el && (el.tagName || '').toLowerCase() === 'input'; }

  // The password box, and the last box before it that could hold a name.
  function pair() {
    var pass = null, boxes = document.querySelectorAll('input[type="password"]');
    for (var p = 0; p < boxes.length; p++) if (seen(boxes[p])) { pass = boxes[p]; break; }
    if (!pass) return null;
    var scope = pass.form || pass.closest('form') || document, all = scope.querySelectorAll('input'), user = null;
    for (var i = 0; i < all.length && all[i] !== pass; i++) {
      var k = kind(all[i]);
      if (k === 'text' || k === 'email' || k === 'tel') user = all[i];
    }
    return { user: user, pass: pass };
  }

  // Strict on purpose, as Safari's and Chrome's are: an account list under
  // a message or a project's name is noise (#631).
  function texty(el) {
    return isInput(el) && !el.disabled && !el.readOnly && ['text', 'email', 'tel', 'number'].indexOf(kind(el)) >= 0 && seen(el);
  }
  function tokens(el) { return (el.getAttribute('autocomplete') || '').toLowerCase().split(/\s+/).filter(Boolean); }
  var foreign = /^(name|honorific-prefix|given-name|additional-name|family-name|honorific-suffix|nickname|organization|organization-title|one-time-code|new-password|street-address|postal-code|country|country-name|bday(-.*)?|sex|url|photo|language|impp|transaction-.*|cc-.*|address-.*|shipping|billing)$/;
  function elsewhere(el) { return tokens(el).some(function (t) { return foreign.test(t); }); }
  function vouched(el) { return tokens(el).some(function (t) { return /^(username|email|webauthn)$/.test(t); }); }
  function words(el) {
    var l = '';
    try { (el.labels || []).forEach(function (x) { l += ' ' + x.textContent; }); } catch (e) {}
    return [el.name, el.id, el.getAttribute('aria-label'), el.getAttribute('placeholder'), l.slice(0, 200)].join(' ').toLowerCase();
  }
  function aside(el) {
    if (/^(q|to|cc|bcc|subject)$/.test(String(el.name || '').toLowerCase())) return true;
    if (/(^|[\s_\-\[])title($|[\s_\-\]])|query|search|otp|one-time|captcha|code|coupon|promo|voucher|gift|recipient|comment|message|description|project/.test(words(el))) return true;
    return !!el.closest('[role="search"], form[role="search"]') || (el.getAttribute('role') === 'combobox' && !vouched(el));
  }
  var who = /user|login|log-in|logon|e-?mail|courriel|correo|identifi|account|compte|cuenta|konto|phone|mobile|t[eé]l[eé]phone|benutzer|anmelde|usuario|utente|utilizador|gebruiker|cpf|fiscal/;
  function claimsName(el) { return vouched(el) || kind(el) === 'email' || who.test(words(el)); }
  var otherForm = /project|title|descri|comment|message|compose|reply|coupon|promo|newsletter|subscri|checkout|cart|basket|shipping|billing|payment|search|contact|feedback|survey|invite|share/;
  var signInPath = /log-?in|log-?on|sign-?in|sign_in|signon|auth(?!or)|account|identif|connexion|connect|session|sso|saml|idp|entrar/;
  var signInButton = /^(next|continue|continuar|suivant|sign ?in|log ?in|login|log on|se connecter|connexion|weiter|anmelden|siguiente|iniciar sesi[oó]n|entrar|acessar|avan[cç]ar|pr[oó]ximo)(\s|$)/;
  function looksLikeSignIn(el, scope) {
    if (/^(login|signin|auth|accounts?|sso|id|identity|secure)\./.test(location.hostname) || signInPath.test(location.pathname.toLowerCase())) return true;
    for (var near = el, up = 0; near && up < 5; up++, near = near.parentElement) {
      var b = near.querySelectorAll('button, input[type="submit"], [role="button"]');
      for (var i = 0; i < b.length && i < 20; i++) {
        var said = ((b[i].tagName === 'INPUT' ? b[i].value : b[i].textContent) || '').replace(/\s+/g, ' ').trim().toLowerCase();
        if (said.length <= 40 && signInButton.test(said)) return true;
      }
      if (near === scope) break;
    }
    return false;
  }
  // The name box of a sign-in that asks for the password on the next page.
  function firstStep(el) {
    if (!texty(el) || aside(el) || elsewhere(el) || !claimsName(el)) return null;
    var scope = el.form || el.closest('form') || document;
    if (scope !== document && otherForm.test([scope.id, scope.getAttribute('name'), scope.getAttribute('action')].join(' ').toLowerCase())) return null;
    var all = scope.querySelectorAll('input'), others = 0;
    for (var i = 0; i < all.length; i++) if (all[i] !== el && texty(all[i]) && !aside(all[i])) others++;
    if (others > (vouched(el) ? 1 : 0)) return null;
    if (!vouched(el) && !scope.querySelector('button, input[type="submit"], [role="button"]')) return null;
    return looksLikeSignIn(el, scope) ? 'wide' : 'exact';
  }
  // What the box the caret is in is offered: null for nothing.
  function judge(el) {
    if (!isInput(el)) return null;
    var both = pair();
    if (both) {
      var fresh = tokens(both.pass).indexOf('new-password') >= 0;
      if (el === both.pass) return fresh ? null : { name: false, wide: true };
      if (el !== both.user || fresh || aside(el) || elsewhere(el)) return null;
      return (el.form && el.form === both.pass.form) || claimsName(el) ? { name: false, wide: true } : null;
    }
    var step = firstStep(el);
    return step ? { name: true, wide: step === 'wide' } : null;
  }
  function named() {
    if (pair()) return null;
    var el = document.activeElement;
    if (firstStep(el)) return el;
    var all = document.querySelectorAll('input'), mine = [];
    for (var i = 0; i < all.length; i++) {
      if (texty(all[i]) && !aside(all[i])) mine.push(all[i]);
      if (mine.length > 6) return null;
    }
    for (var j = 0; j < mine.length; j++) if (firstStep(mine[j])) return mine[j];
    return null;
  }

  // Through the box's own setter, with the events a keystroke fires: a
  // framework that missed them would think the box still empty.
  function put(el, value) {
    filling = true;
    try { el.focus(); } catch (e) {}
    filling = false;
    setValue.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }
  // The app names the document it means (host with its port, as the app keeps
  // it): one that changed since gets nothing.
  function here(scheme, host) {
    return location.protocol === scheme + ':' && location.host.toLowerCase().replace(/^www\./, '') === host;
  }

  // The password last sent, for the app to take once, and only for a while.
  var held = null, heldAt = 0, offered = 0;
  function offer() {
    var both = pair();
    if (!both) {
      var box = named(), name = box ? box.value.trim() : '';
      if (name) { offered = Date.now(); post({ op: 'forms.name', user: name }); }
      return;
    }
    if (!both.pass.value) return;
    offered = Date.now();
    held = both.pass.value;
    heldAt = offered;
    post({ op: 'forms.sent', user: both.user ? both.user.value : '' });
  }

  Object.defineProperty(window, '__officeForms', { value: Object.freeze({
    fill: function (scheme, host, user, password) {
      var both = here(scheme, host) && pair();
      if (!both) return false;
      if (both.user && !both.user.value) put(both.user, user);
      put(both.pass, password);
      return true;
    },
    name: function (scheme, host, user) {
      var box = here(scheme, host) && named();
      if (!box) return false;
      put(box, user);
      return true;
    },
    take: function () {
      var p = Date.now() - heldAt < 10000 ? held : null;
      held = null;
      return p;
    },
    hasPassword: function () { return !!pair(); },
    verdict: function (el) { return judge(el || document.activeElement); },
    sent: function () { if (Date.now() - offered > 2000) offer(); }
  }) });

  document.addEventListener('submit', offer, true);
  document.addEventListener('keydown', function (e) {
    if (e.key !== 'Enter') return;
    var both = pair(), el = document.activeElement;
    if (both ? el === both.pass || el === both.user : firstStep(el)) offer();
  }, true);
  // Plenty of sign-in buttons aren't in a form and never fire submit.
  document.addEventListener('click', function (e) {
    if (e.target && e.target.closest && e.target.closest('button, input[type="submit"], [role="button"]')) setTimeout(offer, 0);
  }, true);

  // The boxes going away without a new page: a sign-in done in place.
  var told = false, looking = null, settling = null;
  function look() {
    looking = null;
    if (!told) { told = !!pair(); return; }
    if (pair()) return;
    told = false;
    clearTimeout(settling);
    settling = setTimeout(function () { if (!pair()) post({ op: 'forms.settled' }); }, 400);
  }
  new MutationObserver(function () { if (!looking) looking = setTimeout(look, 250); })
    .observe(document, { childList: true, subtree: true }); // the document: at its start there is no element yet

  // Where the box with the caret is, in the top page's pixels when a frame
  // can see its way up; said only when it changes.
  var said = null, filling = false;
  function caret() {
    var el = document.activeElement, v = judge(el), rect = null;
    if (v) {
      var r = el.getBoundingClientRect(), x = r.left, y = r.top;
      try {
        for (var w = window; w !== w.top && w.frameElement; w = w.parent) {
          var f = w.frameElement.getBoundingClientRect();
          x += f.left; y += f.top;
        }
      } catch (e) {}
      if (r.width > 0 && r.height > 0) rect = { x: x, y: y, w: r.width, h: r.height };
    }
    var now = JSON.stringify([rect, v]);
    if (now === said) return;
    said = now;
    // The caret moved by a fill of ours: the list it came from is down, and stays so.
    if (!filling) post({ op: 'forms.focus', rect: rect, name: !!(v && v.name) });
  }
  var moving = false;
  function moved() {
    if (moving) return;
    moving = true;
    requestAnimationFrame(function () { moving = false; caret(); });
  }
  addEventListener('scroll', moved, true);
  addEventListener('resize', moved);
  document.addEventListener('focusin', caret, true);
  document.addEventListener('focusout', function () { setTimeout(caret, 0); }, true);
  // A click in a box says it again: the list comes back under it after Esc.
  document.addEventListener('mouseup', function (e) { if (isInput(e.target)) said = null; setTimeout(caret, 0); }, true);
})();
