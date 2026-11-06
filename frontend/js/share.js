// Share photo (Photos tab): a Share button next to Favorite opens a sheet with a QR code that opens
// the photo shown in Immich. The backend sets `shareUrl` (<IMMICH_PUBLIC_URL>/photos/<assetId>) on
// Immich photos when IMMICH_PUBLIC_URL is set; any other photo has none and the button is hidden.
// The QR is a plain link, never a key: whoever scans it signs in to Immich with their own login.
// It's drawn here, offline, by a bundled library. No QR service is ever called.
import { qrcode } from 'qrcode-generator';

(function () {
  'use strict';
  window.ICONS.share = '<path d="M12 3v12"/><path d="M7.5 7.5L12 3l4.5 4.5"/><path d="M8 11H6a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2h-2"/>';

  var QR_PX = 400, QUIET = 4;  // 4 modules of quiet zone, as the QR spec asks for

  // An SVG of the QR code, `px` wide, with whole-pixel modules so it stays sharp.
  function qrSvg(text, px) {
    var qr = qrcode(0, 'M');
    qr.addData(text);
    qr.make();
    var n = qr.getModuleCount(), size = n + QUIET * 2, path = '';
    for (var r = 0; r < n; r++) {
      for (var c = 0; c < n; c++) if (qr.isDark(r, c)) path += 'M' + (c + QUIET) + ' ' + (r + QUIET) + 'h1v1h-1z';
    }
    var w = Math.floor(px / size) * size || size;
    return '<svg xmlns="http://www.w3.org/2000/svg" width="' + w + '" height="' + w + '" viewBox="0 0 ' + size + ' ' + size + '" shape-rendering="crispEdges">' +
      '<rect width="' + size + '" height="' + size + '" fill="#fff"/><path fill="#000" d="' + path + '"/></svg>';
  }

  function current(show) { return show.current && show.current.photos[0]; }

  function open(show) {
    var p = current(show);
    if (!p || !p.shareUrl) return;
    var m = App.photoMeta(p);
    // Hold the slideshow so the QR always matches the photo behind the sheet.
    show.stop();
    App.sheet(
      '<div class="sheet-pad share">' +
        '<div class="share-info">' +
          '<div class="label">Share photo</div>' +
          '<h2 class="serif sheet-h">Scan to open in Immich</h2>' +
          '<div class="share-thumb"><img src="' + App.photoSrc(p) + '" alt=""></div>' +
          '<div class="share-date serif">' + m.date + (m.place ? '<span> · ' + App.esc(m.place) + '</span>' : '') + '</div>' +
          '<p class="share-note">Sign in with your own Immich account to see it.</p>' +
        '</div>' +
        '<div class="share-qr" role="img" aria-label="QR code that opens this photo in Immich">' + qrSvg(p.shareUrl, QR_PX) + '</div>' +
        '<div class="sheet-actions share-actions"><button class="btn btn--solid" data-close>Close</button></div>' +
      '</div>',
      { cls: 'sheet--share', onClose: function () { if (App.current === 'photos') show.start(); } }
    );
  }

  App.share = {
    qrSvg: qrSvg,
    button: function () {
      return '<button class="ph-btn ph-btn--share" data-share hidden>' + App.icon('share', 30) + '<span>Share</span></button>';
    },
    // Wire the Photos view's Share button to its slideshow (once, when the view is built).
    attach: function (el, show) {
      var btn = el.querySelector('[data-share]');
      function sync() { var p = current(show); btn.hidden = !(p && p.shareUrl); }
      var render = show.render;
      show.render = function (slide) { render.call(this, slide); sync(); };
      App.on('data', sync);
      btn.addEventListener('click', function () { open(show); });
      sync();
    }
  };
})();
