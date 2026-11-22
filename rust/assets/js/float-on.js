// The floating video (crate::float), from upstream's Isolate.on: everything
// but the video out of the way. Visibility is inherited, so hiding the body
// and turning it back on for the video alone leaves the player's own
// machinery running untouched, which is what keeps the stream alive where
// cutting the page about would kill it. The page itself then moves into the
// little window, and the video fills it.
(function () {
  function biggest(all) {
    var best = null, area = 0;
    for (var i = 0; i < all.length; i++) {
      var v = all[i];
      if (v.paused || v.ended || v.readyState < 2) continue;
      var box = v.getBoundingClientRect();
      if (box.width * box.height >= area) { area = box.width * box.height; best = v; }
    }
    return best;
  }
  var best = biggest(document.querySelectorAll('video'));
  if (!best) return 'none';
  // Back from the last float before it had landed: that landing is called off.
  window.__searchFloatLanding = null;
  best.setAttribute('data-search-float', '');
  var sheet = document.getElementById('search-float');
  if (!sheet) {
    sheet = document.createElement('style');
    sheet.id = 'search-float';
    (document.head || document.documentElement).appendChild(sheet);
  }
  sheet.textContent = [
    'html.search-floating, html.search-floating body {',
    'background:#000 !important; overflow:hidden !important; margin:0 !important}',
    'html.search-floating body > * { visibility:hidden !important }',
    'html.search-floating [data-search-float] {',
    'visibility:visible !important; position:fixed !important;',
    'left:0 !important; top:0 !important; right:0 !important; bottom:0 !important;',
    'width:100vw !important; height:100vh !important;',
    'max-width:none !important; max-height:none !important;',
    'min-width:0 !important; min-height:0 !important; margin:0 !important; padding:0 !important; border:0 !important;',
    'clip-path:none !important; mask:none !important; -webkit-mask:none !important;',
    // Players that centre the video with a translation would move it out of the window.
    'transform:none !important; translate:none !important; rotate:none !important; scale:none !important;',
    'opacity:1 !important; object-fit:contain !important; z-index:2147483647 !important}',
    // Fixed is only fixed to the window while no ancestor clips it or makes
    // a box of its own for it (a transform, a filter, containment): YouTube's
    // player does, and the video was cut to that box, partly off the window.
    'html.search-floating body :has([data-search-float]) {',
    'overflow:visible !important;',
    'transform:none !important; translate:none !important; rotate:none !important; scale:none !important;',
    'filter:none !important; backdrop-filter:none !important;',
    'perspective:none !important; contain:none !important; container-type:normal !important;',
    'will-change:auto !important; content-visibility:visible !important;',
    'clip-path:none !important; mask:none !important; -webkit-mask:none !important; opacity:1 !important}',
    // Two sets of buttons on one small window is one set too many.
    'html.search-floating [data-search-float]::-webkit-media-controls { display:none !important }'
  ].join('');
  document.documentElement.classList.add('search-floating');

  // The mark is defended: a player that rebuilds itself (a quality change,
  // an ad break) takes the mark with the old element, and the window turns
  // black while holding a live page. Put back on whatever plays now, a
  // quarter of a second after the page changes, as a video starts, and every
  // two seconds for a page that does neither.
  if (window.__searchFloatUnwatch) window.__searchFloatUnwatch();
  clearInterval(window.__searchFloatWatch);
  var remark = function () {
    if (document.querySelector('[data-search-float]')) return;
    var again = biggest(document.querySelectorAll('video'));
    if (again) again.setAttribute('data-search-float', '');
  };
  var queued = null;
  var changes = new MutationObserver(function () {
    if (!queued) queued = setTimeout(function () { queued = null; remark(); }, 250);
  });
  changes.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-search-float'] });
  // Media events don't bubble: caught on the way down.
  document.addEventListener('playing', remark, true);
  window.__searchFloatWatch = setInterval(remark, 2000);
  window.__searchFloatUnwatch = function () {
    changes.disconnect();
    clearTimeout(queued);
    document.removeEventListener('playing', remark, true);
    window.__searchFloatUnwatch = null;
  };
  return { floating: true, w: best.videoWidth, h: best.videoHeight };
})();
