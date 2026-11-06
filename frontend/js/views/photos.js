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

  App.register('photos', {
    show: function (el) {
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
            p.fav = !p.fav;
            heart();
            App.write('state', 'PATCH', '/photos/' + p.id, { fav: p.fav });
            App.toast(p.fav ? 'Added to favorites · it’ll show up more often' : 'Removed from favorites', { icon: 'heart', ms: 2500 });
          }
        });
      }
      el.querySelector('.ph-connect').hidden = !window.DATA.photoAuth;
      el.querySelector('.ph-connect').textContent = window.DATA.photoAuth && window.DATA.photoAuth.status === 'connected' ? 'Immich connection' : 'Connect Immich';
      el.querySelector('.ph-empty').innerHTML = window.DATA.photoAuth
        ? 'No favorite photos yet.<br><i>Connect Immich above, then mark photos as favorites in Immich.</i>'
        : 'No photos yet.<br><i>Add some from your phone at <b>' + location.host + '/upload</b></i>';
      el.querySelector('.ph-empty').hidden = !!window.DATA.photos.length;
      V.show.start();
      if (App.current === 'photos') App.resetIdle();
    },
    resume: function () { this.show(V.el); },
    hide: function () { App.setPhotoFullscreen(false); if (V.show) V.show.stop(); }
  });
})();
