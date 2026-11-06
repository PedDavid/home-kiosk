// Photos: full-bleed editorial slideshow with layout modes.
(function () {
  'use strict';
  var V = { el: null, show: null, layout: 'auto', loud: false };
  var LAYOUTS = [['auto', 'Auto'], ['cover', 'Cover'], ['pair', 'Pair'], ['collage', 'Collage'], ['polaroid', 'Polaroid']];

  var nativeRequested = false, nativeEntered = false, wakeLock = null, wakePending = false;

  function holdScreen() {
    if (!App.photoFullscreen || document.visibilityState !== 'visible' || wakeLock || wakePending || !navigator.wakeLock) return;
    wakePending = true;
    navigator.wakeLock.request('screen').then(function (lock) {
      if (!App.photoFullscreen) { lock.release().catch(function () {}); return; }
      wakeLock = lock;
      lock.addEventListener('release', function () { if (wakeLock === lock) wakeLock = null; });
    }).catch(function () {}).finally(function () { wakePending = false; }); // Kiosk/WebView may already keep the screen awake or lack this API.
  }

  App.setPhotoFullscreen = function (on) {
    on = !!on;
    if (on === App.photoFullscreen || (on && App.current !== 'photos')) return;
    App.photoFullscreen = on;
    App.$('#stage').classList.toggle('is-photo-fullscreen', on);
    App.emit('photo-fullscreen', on);
    App.resetIdle();
    if (on) {
      App.closeSheet();
      App.setMode('day');
      holdScreen();
      if (!document.fullscreenElement && document.documentElement.requestFullscreen) {
        nativeRequested = true;
        try {
          document.documentElement.requestFullscreen().then(function () {
            if (App.photoFullscreen && document.fullscreenElement === document.documentElement) nativeEntered = true;
            if (!App.photoFullscreen && document.fullscreenElement === document.documentElement) {
              document.exitFullscreen().catch(function () {});
            }
          }).catch(function () { nativeRequested = false; });
        } catch (e) { nativeRequested = false; }
      }
      App.$('[data-photo-exit]').focus({ preventScroll: true });
    } else {
      var exitNative = (nativeEntered || nativeRequested) && document.fullscreenElement === document.documentElement;
      nativeEntered = false;
      nativeRequested = false;
      if (wakeLock) { wakeLock.release().catch(function () {}); wakeLock = null; }
      if (exitNative && document.exitFullscreen) document.exitFullscreen().catch(function () {});
      if (App.current === 'photos') App.$('[data-photo-fullscreen]').focus({ preventScroll: true });
    }
  };
  document.addEventListener('fullscreenchange', function () {
    if (nativeRequested && document.fullscreenElement === document.documentElement) nativeEntered = true;
    else if (nativeEntered && !document.fullscreenElement) App.setPhotoFullscreen(false);
  });
  document.addEventListener('keydown', function (e) {
    if (App.photoFullscreen && e.key === 'Escape') { e.preventDefault(); App.setPhotoFullscreen(false); }
  });
  document.addEventListener('visibilitychange', holdScreen);

  function heart() {
    var s = V.show && V.show.current, on = s && s.photos[0].fav;
    var b = V.el.querySelector('[data-fav]');
    b.classList.toggle('is-on', !!on);
    b.querySelector('span').textContent = on ? 'Favorite' : 'Favorite';
  }

  function open(opts) {
    if (!(opts && opts.photo != null && V.show.showPhoto(opts.photo))) V.show.start();
  }

  function setFav(p, fav) {
    p.fav = fav;
    heart();
    App.write('state', 'PATCH', '/photos/' + p.id, { fav: fav });
    App.toast(fav ? 'Added to favorites · it’ll show up more often' : 'Removed from favorites', { icon: 'heart', ms: 2500 });
  }

  // Removing a favorite asks first, and the Remove button has to be held for a second: with
  // Immich it unfavorites the asset in the library too, and a wall frame gets brushed past. The
  // slideshow holds still while the sheet is open, so the question is always about the photo on
  // screen. Cancel, the close button, the scrim, the idle timeout (which closes sheets) and a short
  // tap on Remove all leave the photo a favorite; only a completed hold acts.
  function confirmUnfav(p) {
    V.show.stop();
    var immich = !!(window.DATA.photoAuth && window.DATA.photoAuth.status === 'connected');
    App.sheet('<div class="sheet-pad ph-unfav">' +
      '<img class="ph-unfav-img" src="' + App.photoSrc(p) + '" alt="">' +
      '<div><div class="label">Favorites</div><h2 class="serif sheet-h">Remove from favorites?</h2>' +
      '<p class="sheet-p">' + (immich ? 'It’s also removed from your favorites in Immich.' : 'It will show up less often on this frame.') + '</p>' +
      '<div class="sheet-actions"><button class="btn btn--solid" data-close data-cancel>Cancel</button><button class="btn btn--ghost ph-hold" data-remove><span>Hold to remove</span></button></div>' +
      '<p class="ph-unfav-hint" hidden>Keep holding Remove for a second.</p></div>' +
      '</div>', {
      cls: 'sheet--unfav',
      onMount: function (s, close) {
        s.querySelector('[data-cancel]').focus();
        App.hold(s.querySelector('[data-remove]'), 1000, {
          done: function () { close(); setFav(p, false); },
          cancel: function () { s.querySelector('.ph-unfav-hint').hidden = false; }
        });
      },
      // Resume only if the Photos tab is still showing; leaving it stops the slideshow anyway.
      onClose: function () { if (App.current === 'photos') V.show.start(); }
    });
  }

  App.register('photos', {
    // opts.photo: id of the photo to open on (the home tile passes the one it is showing).
    show: function (el, opts) {
      if (!V.el) {
        V.el = el;
        el.innerHTML = '<div class="photos">' +
          '<div class="ph-stage"></div>' +
          '<button class="ph-btn ph-screen ph-enter" data-photo-fullscreen aria-label="Full screen" title="Full screen">' + App.icon('expand', 24) + '</button>' +
          '<button class="ph-btn ph-screen ph-exit" data-photo-exit aria-label="Exit full screen" title="Exit full screen">' + App.icon('shrink', 24) + '</button>' +
          '<div class="ph-empty serif" hidden>No photos yet.<br><i>Add some from your phone at <b>' + location.host + '/upload</b></i></div>' +
          '<button class="ph-zone ph-zone--l" data-prev aria-label="Previous"></button>' +
          '<button class="ph-zone ph-zone--c" data-loud aria-label="Show details"></button>' +
          '<button class="ph-zone ph-zone--r" data-next aria-label="Next"></button>' +
          '<div class="ph-top"><a class="ph-btn ph-connect" href="/immich" hidden>Connect Immich</a><div class="seg seg--glass">' + LAYOUTS.map(function (l) {
            return '<button data-layout="' + l[0] + '" class="' + (l[0] === V.layout ? 'is-on' : '') + '">' + l[1] + '</button>';
          }).join('') + '</div></div>' +
          '<div class="ph-bottom">' +
          '  <button class="ph-btn" data-prev>' + App.icon('left', 30) + '</button>' +
          '  <button class="ph-btn ph-btn--fav" data-fav>' + App.icon('heart', 30) + '<span>Favorite</span></button>' +
          '  <button class="ph-btn" data-next>' + App.icon('right', 30) + '</button>' +
          '</div></div>';
        V.show = new App.Slideshow(el.querySelector('.ph-stage'), { layout: V.layout, interval: 15000, onChange: function () { setTimeout(heart); } });
        el.addEventListener('click', function (e) {
          var t = e.target.closest('button');
          if (!t) return;
          if (t.hasAttribute('data-photo-fullscreen')) { App.setPhotoFullscreen(true); return; }
          if (t.hasAttribute('data-photo-exit')) { App.setPhotoFullscreen(false); return; }
          if (t.hasAttribute('data-next')) V.show.next();
          if (t.hasAttribute('data-prev')) V.show.prev();
          if (t.hasAttribute('data-loud')) { V.loud = !V.loud; el.querySelector('.photos').classList.toggle('is-loud', V.loud); }
          if (t.dataset.layout) {
            V.layout = t.dataset.layout;
            App.$$('[data-layout]', el).forEach(function (b) { b.classList.toggle('is-on', b === t); });
            V.show.setLayout(V.layout);
          }
          if (t.hasAttribute('data-fav')) {
            var p = V.show.current && V.show.current.photos[0];
            if (!p) return;
            if (p.fav) confirmUnfav(p); else setFav(p, true);
          }
        });
      }
      el.querySelector('.ph-connect').hidden = !window.DATA.photoAuth;
      el.querySelector('.ph-connect').textContent = window.DATA.photoAuth && window.DATA.photoAuth.status === 'connected' ? 'Immich connection' : 'Connect Immich';
      el.querySelector('.ph-empty').innerHTML = window.DATA.photoAuth
        ? 'No favorite photos yet.<br><i>Connect Immich above, then mark photos as favorites in Immich.</i>'
        : 'No photos yet.<br><i>Add some from your phone at <b>' + location.host + '/upload</b></i>';
      el.querySelector('.ph-empty').hidden = !!window.DATA.photos.length;
      open(opts);
      if (App.current === 'photos') App.resetIdle();
    },
    resume: function (el, opts) { this.show(V.el, opts); },
    hide: function () { App.setPhotoFullscreen(false); if (V.show) V.show.stop(); }
  });
})();
