// Photos: full-bleed editorial slideshow with layout modes.
(function () {
  'use strict';
  var V = { el: null, show: null, layout: 'auto', loud: false };
  var LAYOUTS = [['auto', 'Auto'], ['cover', 'Cover'], ['pair', 'Pair'], ['collage', 'Collage'], ['polaroid', 'Polaroid']];

  function heart() {
    var s = V.show && V.show.current, on = s && s.photos[0].fav;
    var b = V.el.querySelector('[data-fav]');
    b.classList.toggle('is-on', !!on);
    b.querySelector('span').textContent = on ? 'Favourite' : 'Favourite';
  }

  function setFav(p, fav) {
    p.fav = fav;
    heart();
    App.write('state', 'PATCH', '/photos/' + p.id, { fav: fav });
    var immich = !!(window.DATA && window.DATA.immich);
    App.toast(fav ? 'Added to favourites · it’ll show up more often' : 'Removed from favourites' + (immich ? ' · and in Immich' : ''), { icon: 'heart', ms: 2500 });
  }

  // Adding a favourite is one tap; removing one is press-and-hold. With Immich, removing also
  // unfavourites the asset in the library, and a wall frame gets brushed past. The photo is
  // captured when the press starts; if the slide changes mid-hold, nothing is removed.
  // A short tap on a favourite only explains the gesture.
  function holdToRemove(btn) {
    var held = null;
    App.hold(btn, 1000, {
      enabled: function () { held = V.show.current && V.show.current.photos[0]; return !!(held && held.fav); },
      done: function () {
        V.lastHold = Date.now();
        var cur = V.show.current && V.show.current.photos[0];
        if (held && held.fav && held === cur) setFav(held, false);
      },
      cancel: function () { App.toast('Hold to remove from favourites', { icon: 'heart', ms: 2200 }); }
    });
  }

  App.register('photos', {
    show: function (el) {
      if (!V.el) {
        V.el = el;
        el.innerHTML = '<div class="photos">' +
          '<div class="ph-stage"></div>' +
          '<div class="ph-empty serif" hidden>No photos yet.<br><i>Add some from your phone at <b>' + location.host + '/upload</b></i></div>' +
          '<button class="ph-zone ph-zone--l" data-prev aria-label="Previous"></button>' +
          '<button class="ph-zone ph-zone--c" data-loud aria-label="Show details"></button>' +
          '<button class="ph-zone ph-zone--r" data-next aria-label="Next"></button>' +
          '<div class="ph-top"><div class="seg seg--glass">' + LAYOUTS.map(function (l) {
            return '<button data-layout="' + l[0] + '" class="' + (l[0] === V.layout ? 'is-on' : '') + '">' + l[1] + '</button>';
          }).join('') + '</div></div>' +
          '<div class="ph-bottom">' +
          '  <button class="ph-btn" data-prev>' + App.icon('left', 30) + '</button>' +
          '  <button class="ph-btn ph-btn--fav" data-fav>' + App.icon('heart', 30) + '<span>Favourite</span></button>' +
          '  <button class="ph-btn" data-next>' + App.icon('right', 30) + '</button>' +
          '</div></div>';
        holdToRemove(el.querySelector('[data-fav]'));
        V.show = new App.Slideshow(el.querySelector('.ph-stage'), { layout: V.layout, interval: 15000, onChange: function () { setTimeout(heart); } });
        el.addEventListener('click', function (e) {
          var t = e.target.closest('button');
          if (!t) return;
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
            // Removing is press-and-hold (holdToRemove); ignore the click that ends a completed hold.
            if (!p.fav && Date.now() - (V.lastHold || 0) > 800) setFav(p, true);
          }
        });
      }
      el.querySelector('.ph-empty').hidden = !!window.DATA.photos.length;
      V.show.start();
    },
    resume: function () { V.el.querySelector('.ph-empty').hidden = !!window.DATA.photos.length; V.show.start(); },
    hide: function () { if (V.show) V.show.stop(); }
  });
})();
