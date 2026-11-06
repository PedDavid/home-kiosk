// Share photo (Photos tab): an icon-only Share button beside the full-screen button, there in full
// screen too, opens a sheet with a QR code that opens the photo shown in the Immich phone app
// (`shareAppUrl`, immich://asset?id=<assetId>) or, switched to Web, in Immich on the web
// (`shareUrl`, <HQ_IMMICH_PUBLIC_URL>/photos/<assetId>). The backend sets both on Immich photos
// when HQ_IMMICH_PUBLIC_URL is set; any other photo has neither and the button is hidden.
// The QR is a plain link, never a key: whoever scans it signs in to Immich with their own login.
// It's drawn here, offline, by a bundled library. No QR service is ever called.
import { qrcode } from 'qrcode-generator';

(function () {
  'use strict';
  window.ICONS.share = '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><path d="M8.6 13.5l6.8 4M15.4 6.5l-6.8 4"/>';

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

  // The open share sheet's close(), so it can time out: the idle timer that normally closes sheets
  // is off in photo full screen, so the sheet closes itself after the same "back to Today" delay.
  var closeOpen = null;
  App.on('second', function () {
    if (!closeOpen) return;
    if (!document.querySelector('#sheetLayer .sheet--share')) closeOpen = null;  // replaced by another sheet
    else if (App.idleFor() >= App.idle.home) closeOpen();
  });

  // What the QR opens: the Immich phone app (immich://asset?id=…) or the web page. Remembered per
  // screen; the app is the default since household phones have it.
  var MODES = {
    app: { label: 'Immich app', note: 'Opens it in the Immich app on your phone. No app? Switch to Web.' },
    web: { label: 'Web', note: 'Opens it in your phone’s browser. Sign in with your own Immich account.' }
  };
  var mode = 'app';
  try { if (localStorage.getItem('hq-share-qr') === 'web') mode = 'web'; } catch (e) {}

  function qrFor(p) { return mode === 'app' && p.shareAppUrl ? p.shareAppUrl : p.shareUrl; }

  // The web link as text. Clickable in a browser; on the frame (the kiosk app) it's plain text,
  // because following a link there would take the dashboard itself away.
  function webLink(p) {
    var text = App.esc(p.shareUrl.replace(/^https?:\/\//, ''));
    return window.HomeHQKiosk
      ? '<span class="share-link">' + text + '</span>'
      : '<a class="share-link" href="' + App.esc(p.shareUrl) + '" target="_blank" rel="noopener noreferrer">' + text + '</a>';
  }

  function open(show) {
    var p = current(show);
    if (!p || !p.shareUrl) return;
    var m = App.photoMeta(p);
    // Hold the slideshow so the QR always matches the photo behind the sheet.
    show.stop();
    App.resetIdle();
    closeOpen = App.sheet(
      '<div class="sheet-pad share">' +
        '<div class="share-info">' +
          '<div class="label">Share photo</div>' +
          '<h2 class="serif sheet-h">Scan to open in Immich</h2>' +
          (p.shareAppUrl ? '<div class="seg share-mode">' + Object.keys(MODES).map(function (k) {
            return '<button data-qr="' + k + '">' + MODES[k].label + '</button>';
          }).join('') + '</div>' : '') +
          '<div class="share-thumb"><img src="' + App.photoSrc(p) + '" alt=""></div>' +
          '<div class="share-date serif">' + m.date + (m.place ? '<span> · ' + App.esc(m.place) + '</span>' : '') + '</div>' +
          '<p class="share-note"></p>' +
        '</div>' +
        '<div class="share-qr" role="img"></div>' +
        '<div class="sheet-actions share-actions">' + webLink(p) + '<button class="btn btn--solid" data-close>Close</button></div>' +
      '</div>',
      {
        cls: 'sheet--share',
        onClose: function () { closeOpen = null; if (App.current === 'photos') show.start(); },
        onMount: function (sheet) {
          function draw() {
            var app = mode === 'app' && !!p.shareAppUrl;
            sheet.querySelector('.share-qr').innerHTML = qrSvg(qrFor(p), QR_PX);
            sheet.querySelector('.share-qr').setAttribute('aria-label', 'QR code that opens this photo in ' + (app ? 'the Immich app' : 'Immich on the web'));
            sheet.querySelector('.share-note').textContent = MODES[app ? 'app' : 'web'].note;
            App.$$('[data-qr]', sheet).forEach(function (b) { b.classList.toggle('is-on', b.dataset.qr === mode); });
          }
          App.$$('[data-qr]', sheet).forEach(function (b) {
            b.addEventListener('click', function () {
              mode = b.dataset.qr;
              try { localStorage.setItem('hq-share-qr', mode); } catch (e) {}
              draw();
            });
          });
          draw();
        }
      }
    );
  }

  App.share = {
    qrSvg: qrSvg,
    button: function () {
      return '<button class="ph-btn ph-screen ph-share" data-share hidden aria-label="Share photo" title="Share photo">' + App.icon('share', 24) + '</button>';
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
