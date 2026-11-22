// The floating video back in its tab (crate::float), from upstream's
// Isolate.off: everything as it was, once the page is laid out at the tab's
// size again. Put back at once, while the page still had the little window's
// size, the player fitted the video to that, and the tab's first frames
// showed it so before it grew again.
(function () {
  // The engine may have put the video in a picture-in-picture window of its
  // own as well (some players ask for it): leaving one and not the other
  // leaves you with two.
  try {
    if (document.pictureInPictureElement && document.exitPictureInPicture) document.exitPictureInPicture();
  } catch (e) {}
  var root = document.documentElement;
  var landing = window.__searchFloatLanding = {};
  function put() {
    // Floated again in the meantime: that is the float's now.
    if (window.__searchFloatLanding !== landing) return;
    window.__searchFloatLanding = null;
    if (window.__searchFloatUnwatch) window.__searchFloatUnwatch();
    clearInterval(window.__searchFloatWatch);
    window.__searchFloatWatch = null;
    root.classList.remove('search-floating');
    var sheet = document.getElementById('search-float');
    if (sheet) sheet.textContent = '';
    var video = document.querySelector('[data-search-float]');
    if (video) video.removeAttribute('data-search-float');
  }
  // Each frame until the page is laid out at another size than the little
  // window's; a page not drawn (its tab behind another) is put back by the clock.
  var wide = innerWidth, high = innerHeight, began = Date.now();
  (function frame() {
    if (window.__searchFloatLanding !== landing) return;
    if (innerWidth !== wide || innerHeight !== high || Date.now() - began > 400) return put();
    requestAnimationFrame(frame);
  })();
  setTimeout(put, 1000);
  return 'landing';
})();
