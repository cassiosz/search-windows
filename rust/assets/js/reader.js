// Ported from Search (Office Commun), MIT (Reader.swift)
// Reading mode: the article, and nothing that was arranged around it.
// The piece of the page carrying the most prose, punished for every link inside it.
// Run with ExecuteScript; the result is 'read' (done / already reading) or 'none' (no article found).
// Optional: set window.__officeReaderLook = 'light' | 'dark' | 'system' (default 'system') before running.
(function () {
  var bridge = window.chrome && window.chrome.webview;
  function post(msg) { try { if (bridge) bridge.postMessage(msg); } catch (e) {} }

  var dark = 'color-scheme:dark;--r-ground:#1c1c1c;--r-ink:#e0e0e0;--r-quiet:#8a8a8a;' +
    '--r-aside:#a8a8a8;--r-rule:#333;--r-code:#2a2a2a;--r-picture:brightness(.92)';
  var sans = '"Segoe UI Variable Text","Segoe UI",system-ui,sans-serif';

  var sheet = [
    ':root{color-scheme:light;--r-ground:#fff;--r-ink:#171717;--r-quiet:#a3a3a3;',
    '--r-aside:#555;--r-rule:#e8e8e8;--r-code:#f5f5f5;--r-picture:none}',
    '@media (prefers-color-scheme:dark){:root:not([data-office-look=light]){' + dark + '}}',
    ':root[data-office-look=dark]{' + dark + '}',
    'html,body{background:var(--r-ground) !important;margin:0 !important;padding:0 !important}',
    '#office-reader{max-width:38em;margin:0 auto;padding:72px 24px 160px;',
    'font:400 18px/1.72 ui-serif,Georgia,"Times New Roman",serif;color:var(--r-ink)}',
    '#office-reader h1{font:600 30px/1.24 ' + sans + ';',
    'margin:0 0 8px;letter-spacing:-0.01em}',
    '#office-reader .office-from{font:400 12px/1 ' + sans + ';color:var(--r-quiet);',
    'margin:0 0 40px;text-transform:uppercase;letter-spacing:.06em}',
    '#office-reader p{margin:0 0 1.35em}',
    '#office-reader img,#office-reader video,#office-reader iframe{max-width:100%;',
    'height:auto;border-radius:6px;margin:1.6em 0;display:block}',
    '#office-reader img{filter:var(--r-picture)}',
    '#office-reader iframe{width:100%;aspect-ratio:16/9;height:auto;border:0}',
    '#office-reader svg{max-width:100%}',
    '#office-reader iframe.office-chart{aspect-ratio:auto}',
    '#office-reader figure{margin:1.8em 0}',
    '#office-reader figcaption{font:400 13px/1.5 ' + sans + ';',
    'color:var(--r-quiet);margin-top:.6em}',
    '#office-reader figcaption > * + *{margin-left:.35em}',
    '#office-reader .office-above{font:400 14px/1.55 ' + sans + ';',
    'color:var(--r-aside);margin:0 0 1.6em}',
    '#office-reader .office-above ul{padding-left:1.2em}',
    '#office-reader .office-above h2{margin:.4em 0}',
    '#office-reader a{color:var(--r-ink);text-underline-offset:3px}',
    '#office-reader h2,#office-reader h3{font:600 20px/1.3 ' + sans + ';',
    'margin:2em 0 .6em}',
    '#office-reader h4,#office-reader h5,#office-reader h6{font:600 16px/1.4 ' + sans + ';',
    'margin:1.8em 0 .5em}',
    '#office-reader pre,#office-reader code,#office-reader kbd{font-family:ui-monospace,Consolas,monospace;font-size:14px}',
    '#office-reader pre{background:var(--r-code);padding:14px;border-radius:8px;overflow:auto}',
    '#office-reader blockquote{margin:1.6em 0;padding-left:1.2em;',
    'border-left:2px solid var(--r-rule);color:var(--r-aside)}',
    '#office-reader ul,#office-reader ol{margin:0 0 1.35em;padding-left:1.4em}',
    '#office-reader ul{list-style:disc outside}',
    '#office-reader ol{list-style:decimal outside}',
    '#office-reader li>ul{list-style-type:circle}',
    '#office-reader :not(pre)>code{background:var(--r-code);padding:.1em .35em;border-radius:4px}',
    '#office-reader li{margin:.3em 0}',
    '#office-reader li>ul,#office-reader li>ol{margin-bottom:0}',
    '#office-reader hr{border:0;border-top:1px solid var(--r-rule);margin:2.4em 0}',
    '#office-reader .office-wide{overflow-x:auto;margin:1.6em 0}',
    '#office-reader table{border-collapse:collapse;font:400 15px/1.5 ' + sans + '}',
    '#office-reader th,#office-reader td{border-bottom:1px solid var(--r-rule);padding:8px 12px 8px 0;',
    'text-align:left;vertical-align:top}',
    '#office-reader th{font-weight:600}',
    '#office-reader .office-part{font:600 16px/1.4 ' + sans + ';margin:1.8em 0 .5em}',
    '#office-reader details{margin:0 0 1.35em}',
    '#office-reader summary{cursor:pointer}',
    // The light / dark / system switch, quiet until the pointer comes to it.
    '#office-look{position:fixed;top:14px;right:14px;display:flex;gap:2px;padding:3px;',
    'background:var(--r-ground);border:1px solid var(--r-rule);border-radius:999px;',
    'opacity:.55;transition:opacity .12s;z-index:2147483647;font:500 11px ' + sans + '}',
    '#office-look:hover{opacity:1}',
    '#office-look button{all:unset;cursor:pointer;color:var(--r-quiet);padding:4px 9px;border-radius:999px}',
    '#office-look button[aria-pressed=true]{color:var(--r-ink);background:var(--r-code)}'
  ].join('');

  function setLook(look) {
    var root = document.documentElement;
    if (look === 'light' || look === 'dark') root.setAttribute('data-office-look', look);
    else root.removeAttribute('data-office-look');
    var btns = document.querySelectorAll('#office-look button');
    var cur = root.getAttribute('data-office-look') || 'system';
    for (var i = 0; i < btns.length; i++) {
      btns[i].setAttribute('aria-pressed', String(btns[i].getAttribute('data-look') === cur));
    }
  }

  // Puts the reading page in place of the page. The page's own stylesheets go first: left
  // in, they reach the reading page through every rule written for a bare element and every
  // font the page loaded. Search's own (id office-*) stay; adoptedStyleSheets are left alone.
  function present(wrap, title, from) {
    var chosen = typeof window.__officeReaderLook === 'string' ? window.__officeReaderLook : 'system';
    var theirs = document.querySelectorAll('style, link[rel~="stylesheet" i]');
    for (var i = 0; i < theirs.length; i++) {
      if ((theirs[i].id || '').indexOf('office-') !== 0) theirs[i].remove();
    }
    document.documentElement.removeAttribute('style');
    document.body.removeAttribute('style');
    var style = document.createElement('style');
    style.id = 'office-reader-sheet';
    style.textContent = sheet;
    var top = document.createElement('h1');
    top.textContent = title;
    var line = document.createElement('p');
    line.className = 'office-from';
    line.textContent = from;
    wrap.id = 'office-reader';
    document.body.innerHTML = '';
    (document.head || document.documentElement).appendChild(style);
    wrap.insertBefore(line, wrap.firstChild);
    wrap.insertBefore(top, wrap.firstChild);
    document.body.appendChild(wrap);

    var pick = document.createElement('div');
    pick.id = 'office-look';
    ['light', 'system', 'dark'].forEach(function (look) {
      var b = document.createElement('button');
      b.setAttribute('data-look', look);
      b.textContent = look === 'light' ? 'Light' : look === 'dark' ? 'Dark' : 'Auto';
      b.onclick = function () {
        setLook(look);
        post({ op: 'reader.look', look: look });
      };
      pick.appendChild(b);
    });
    document.body.appendChild(pick);
    setLook(chosen);
    window.scrollTo(0, 0);
  }

  function prose(el) {
    var paragraphs = el.querySelectorAll('p');
    if (paragraphs.length < 2) return 0;
    var letters = 0;
    for (var i = 0; i < paragraphs.length; i++) {
      letters += (paragraphs[i].innerText || '').length;
    }
    if (letters < 400) return 0;
    // A rail of related links has plenty of text and nothing to read. What tells it from an
    // article is how much of it the links are: a rail is almost all link, an article that
    // cites its sources a tenth of itself at most. Both halves measured with textContent.
    var whole = (el.textContent || '').length;
    if (!whole) return 0;
    var inside = 0, links = el.querySelectorAll('a');
    for (var j = 0; j < links.length; j++) {
      inside += (links[j].textContent || '').length;
    }
    return letters * (1 - Math.min(inside / whole, 1));
  }

  function best() {
    var candidates = document.querySelectorAll(
      'article, main, [role="main"], .post, .entry, .article, .content, #content, div, section'
    );
    var top = null, mark = 0;
    for (var i = 0; i < candidates.length; i++) {
      var el = candidates[i], score = prose(el);
      if (!score) continue;
      if (!top) { top = el; mark = score; continue; }
      // Between a box and one it holds, the smaller is taken unless the larger is clearly
      // better (a quarter ahead), not merely bigger: the box around an article and its
      // comments would otherwise bring the comments in.
      if (top.contains(el)) {
        if (score * 1.25 > mark) { top = el; mark = score; }
      } else if (el.contains(top)) {
        if (score > mark * 1.25) { top = el; mark = score; }
      } else if (score > mark) {
        top = el; mark = score;
      }
    }
    return top;
  }

  function read() {
    // Already an article: going back can bring one as it was left.
    if (document.getElementById('office-reader')) return 'read';
    if (!document.body) return 'none';

    var article = best();
    if (!article) return 'none';

    // The photo an article opens with isn't always in it (a topper beside the <article>):
    // the first large picture before the text comes too, no more than four boxes out,
    // and never in a menu, a rail or a footer.
    var lead = null;
    for (var up = article.parentElement, depth = 0;
         up && up !== document.body && depth < 4 && !lead;
         up = up.parentElement, depth++) {
      var shown = up.querySelectorAll('figure, picture');
      for (var l = 0; l < shown.length && !lead; l++) {
        var face = shown[l].querySelector('img');
        if (face && article.compareDocumentPosition(shown[l]) === Node.DOCUMENT_POSITION_PRECEDING &&
            !shown[l].closest('nav, aside, footer, [role="banner"]') &&
            face.getBoundingClientRect().width >= 400) lead = shown[l];
      }
    }

    // A chart's frame is as tall as its own script made it, and that script won't run in
    // the reader: the height is read while the page shows it.
    var framed = article.querySelectorAll('iframe');
    for (var z = 0; z < framed.length; z++) {
      framed[z].setAttribute('data-office-height', Math.round(framed[z].getBoundingClientRect().height));
    }

    // The size a drawing in the text is given (an icon as often as not) goes with the
    // page's styles: left alone, an <svg> fills the column. The drawn size is written on it.
    var drawings = article.querySelectorAll('svg');
    for (var dr = 0; dr < drawings.length; dr++) {
      if (drawings[dr].parentElement.closest('svg')) continue;
      var drawn = drawings[dr].getBoundingClientRect();
      drawings[dr].setAttribute('width', Math.round(drawn.width));
      drawings[dr].setAttribute('height', Math.round(drawn.height));
    }

    // Parts of the head that sit between the headline and the text, outside the part with the
    // most prose (byline, takeaways, lede, subheading), in the page's order; only those drawn
    // on the page, never one from a menu, a rail or a footer.
    var heading = document.querySelector('h1');
    var above = [];
    if (heading) {
      var found = document.querySelectorAll(
        '[class*="byline" i],[class*="takeaways" i],[class*="lede" i],h2'
      );
      for (var a = 0; a < found.length; a++) {
        var part = found[a];
        if (part.contains(article) || article.contains(part) ||
            above.some(function (o) { return o.contains(part); })) continue;
        if (!(heading.compareDocumentPosition(part) & Node.DOCUMENT_POSITION_FOLLOWING)) continue;
        if (!(article.compareDocumentPosition(part) & Node.DOCUMENT_POSITION_PRECEDING)) continue;
        if (part.closest('nav, aside, footer, [role="banner"]') || !part.getClientRects().length) continue;
        above.push(part);
      }
    }

    // Every picture is resolved while the real page is still standing: currentSrc is what the
    // browser chose and loaded after srcset, sizes and <picture>; written back as a plain src
    // it is the same image the page showed, not a lazy placeholder.
    var late = ['data-src', 'data-original', 'data-lazy-src', 'data-lazy',
                'data-full-src', 'data-hi-res-src', 'data-image', 'data-echo'];
    var pictures = Array.prototype.slice.call(article.querySelectorAll('img'));
    if (lead) pictures = pictures.concat(Array.prototype.slice.call(lead.querySelectorAll('img')));
    above.forEach(function (o) {
      pictures = pictures.concat(Array.prototype.slice.call(o.querySelectorAll('img')));
    });
    for (var i = 0; i < pictures.length; i++) {
      var picture = pictures[i];
      picture.setAttribute('loading', 'eager');
      var real = picture.currentSrc || picture.getAttribute('src') || '';
      // A one-pixel placeholder counts as nothing.
      if (!real || real.indexOf('data:image') === 0 || picture.naturalWidth <= 2) {
        for (var k = 0; k < late.length; k++) {
          var kept0 = picture.getAttribute(late[k]);
          if (kept0) { real = kept0; break; }
        }
      }
      if (real) picture.setAttribute('src', real);
      var lateSet = picture.getAttribute('data-srcset');
      if (lateSet && !picture.getAttribute('srcset')) {
        picture.setAttribute('srcset', lateSet);
      }
    }

    // What the page's own styles hide, seen while they still apply: the reading page goes
    // without them, and a label meant only for screen readers, a folded menu or the mobile
    // copy of a table would all come into view. Not a box that holds paragraphs, though:
    // an article's folded-away rest is still the article.
    var inherent = /^(SOURCE|TRACK|PARAM|AREA|TEMPLATE|BR|WBR|SCRIPT|STYLE|NOSCRIPT)$/;
    var all = [];
    [article].concat(above).forEach(function (root) {
      all = all.concat(Array.prototype.slice.call(root.querySelectorAll('*')));
    });
    for (var h = 0; h < all.length; h++) {
      var el = all[h];
      if (inherent.test(el.tagName) || el.closest('details:not([open])')) continue;
      var look = getComputedStyle(el);
      var gone = look.display === 'none' || look.visibility === 'hidden';
      if (!gone && (look.overflow === 'hidden' || look.clip !== 'auto')) {
        var box = el.getBoundingClientRect();
        // The usual way of hiding text from sight but not from a screen reader.
        gone = box.width <= 1 && box.height <= 1;
      }
      if (gone && !(look.display === 'none' && el.querySelector('p'))) el.setAttribute('data-office-hidden', '');
    }

    var title = (heading && heading.innerText.trim()) || document.title;

    var wrap = document.createElement('div');
    wrap.innerHTML = article.innerHTML;
    for (var o = above.length - 1; o >= 0; o--) {
      var copy = above[o].cloneNode(true);
      copy.classList.add('office-above');
      wrap.insertBefore(copy, wrap.firstChild);
    }

    var hidden = wrap.querySelectorAll('[data-office-hidden]');
    for (var d = 0; d < hidden.length; d++) hidden[d].remove();
    // A style written on an element is the page's styling too.
    var styled = wrap.querySelectorAll('[style]');
    for (var y = 0; y < styled.length; y++) styled[y].removeAttribute('style');
    // The photo comes without the topper's own classes and styles.
    if (lead) {
      var opening = lead.cloneNode(true);
      var parts = [opening].concat(Array.prototype.slice.call(opening.querySelectorAll('*')));
      for (var x = 0; x < parts.length; x++) {
        parts[x].removeAttribute('class');
        parts[x].removeAttribute('style');
      }
      var after = wrap.firstChild;
      while (after && after.classList && after.classList.contains('office-above')) after = after.nextSibling;
      wrap.insertBefore(opening, after);
    }

    // A picture in a button (a click to see it larger) is the picture: the button goes,
    // what it holds stays.
    var pressed = wrap.querySelectorAll('button');
    for (var b = 0; b < pressed.length; b++) {
      if (!pressed[b].querySelector('img, picture, video')) continue;
      while (pressed[b].firstChild) pressed[b].parentNode.insertBefore(pressed[b].firstChild, pressed[b]);
      pressed[b].remove();
    }

    // What was arranged around the words rather than being part of them. Not header: an
    // article's opening image lives there as often as not. A video.js player's controls go
    // too; the video stays.
    var clutter = wrap.querySelectorAll(
      'script,style,noscript,form,nav,aside,footer,button,input,select,textarea,' +
      '[role="complementary"],[role="navigation"],[role="banner"],[aria-hidden="true"],' +
      '.video-js > [class*="vjs-"]:not(video)'
    );
    for (var c = 0; c < clutter.length; c++) clutter[c].remove();

    // A box inviting you to a newsletter, set in the middle of the text. Only a short one.
    var invites = wrap.querySelectorAll('[class*="newsletter" i]');
    for (var n = 0; n < invites.length; n++) {
      if (invites[n].querySelectorAll('p').length < 3) invites[n].remove();
    }

    // Embedded video is part of the article, and so is a chart or a map from a chart maker,
    // at the height it had. Any other frame is not: an advertisement, a widget.
    var players = /youtube|youtu\.be|vimeo|dailymotion|loom\.com|streamable|wistia|ted\.com/i;
    var charts = /flourish|flo\.uri\.sh|datawrapper|dwcdn\.net|infogram|tableau|ig\.ft\.com|google\.com\/maps/i;
    var frames = wrap.querySelectorAll('iframe');
    for (var f = 0; f < frames.length; f++) {
      var where = frames[f].getAttribute('src') || frames[f].getAttribute('data-src') || '';
      if (players.test(where)) {
        frames[f].setAttribute('src', where);
        frames[f].removeAttribute('height');
        frames[f].removeAttribute('width');
      } else if (where && charts.test(where)) {
        var tall = +frames[f].getAttribute('data-office-height');
        frames[f].setAttribute('src', where);
        frames[f].setAttribute('style', 'height:' + (tall > 60 ? tall : 420) + 'px');
        frames[f].className = 'office-chart';
      } else {
        frames[f].remove();
      }
    }

    // A video's poster, which its player also shows as a picture beside it: once is enough.
    var videos = wrap.querySelectorAll('video'), posters = [];
    for (var v = 0; v < videos.length; v++) {
      if (videos[v].poster) posters.push(videos[v].poster);
    }
    var stills = wrap.querySelectorAll('img');
    for (var t = 0; t < stills.length; t++) {
      if (posters.indexOf(stills[t].src) >= 0 && !stills[t].closest('video')) stills[t].remove();
    }

    // A video its player fed from the page itself, through a blob: address, has nothing to
    // play once that player is gone. It plays from the plain stream named beside that, with
    // controls and never by itself; with none, it is its poster; with no poster, it goes.
    for (var q = 0; q < videos.length; q++) {
      var video = videos[q], fed = /^blob:/.test(video.getAttribute('src') || ''), plain = '';
      var named = video.querySelectorAll('source');
      for (var m = 0; m < named.length; m++) {
        if (/^blob:/.test(named[m].getAttribute('src') || '')) fed = true;
        else if (!plain && /^https?:/.test(named[m].src)) plain = named[m].src;
      }
      if (!fed) continue;
      if (plain) {
        for (var r = 0; r < named.length; r++) named[r].remove();
        video.setAttribute('preload', 'metadata');
        video.setAttribute('src', plain);
        video.setAttribute('controls', '');
        video.setAttribute('playsinline', '');
        video.removeAttribute('autoplay');
        var player = video.parentNode.closest('.video-js');
        if (player) player.parentNode.replaceChild(video, player);
      } else if (video.poster) {
        var still = document.createElement('img');
        still.setAttribute('src', video.poster);
        video.parentNode.replaceChild(still, video);
      } else {
        video.remove();
      }
    }

    // A picture with nothing behind it is a broken icon, worse than no picture at all.
    var kept = wrap.querySelectorAll('img');
    for (var g = 0; g < kept.length; g++) {
      var src = kept[g].getAttribute('src') || '';
      if (!src || src.indexOf('data:image') === 0) kept[g].remove();
    }

    // A box whose content went goes too, rather than stay as an empty band.
    var boxes = wrap.querySelectorAll('figure, picture, div, section, p');
    for (var e = boxes.length - 1; e >= 0; e--) {
      if (!boxes[e].textContent.trim() &&
          !boxes[e].querySelector('img, video, audio, iframe, svg, canvas, table')) boxes[e].remove();
    }

    // The article's own heading goes when it only says the title again.
    var own = wrap.querySelector('h1');
    if (own && own.textContent.replace(/\s+/g, ' ').trim().toLowerCase() ===
        title.replace(/\s+/g, ' ').toLowerCase()) own.remove();

    present(wrap, title, location.host.replace(/^www\./, ''));
    return 'read';
  }

  try { return read(); } catch (err) { post({ op: 'reader.trouble', trouble: String(err) }); return 'none'; }
})();
