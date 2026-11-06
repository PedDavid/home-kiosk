// Ambient (idle photo frame) and Night (dim red clock) overlays.
(function () {
  'use strict';
  var D = window.DATA, $ = App.$;
  var amb = { show: null }, shiftTimer = null;

  function nightInfo() {
    var n = App.now(), tomorrow = App.addDays(n, n.getHours() < 6 ? 0 : 1);
    var first = App.timedOn(tomorrow)[0], w = App.weatherNow();
    var locks = D.homeOnline ? D.devices.filter(function (d) { return d.type === 'lock' || d.type === 'garage'; }) : [];
    var open = locks.filter(function (d) { return d.type === 'lock' ? !d.locked : d.state !== 'closed'; });
    return '<div class="nt-time num">' + App.fmt.time(n, false) + '</div>' +
      '<div class="nt-line">' + (w ? App.icon(w.cond, 26) + ' ' + w.temp + '° · ' : '') + (first ? 'First up ' + App.fmt.rel(first.start).toLowerCase() + ': <b>' + App.esc(first.title) + '</b> ' + App.fmt.time(first.start) + ' · ' + (App.isFamily(first) ? 'Everyone' : App.person(first.people[0]).name) : 'Nothing early tomorrow') + '</div>' +
      (locks.length ? '<div class="nt-lock">' + App.icon(open.length ? 'unlock' : 'lock', 20) + (open.length ? ' ' + open.map(function (d) { return App.esc(d.name); }).join(', ') + ' open' : ' All doors locked') + '</div>' : '');
  }

  function paint() {
    if (App.mode === 'night') $('#night .nt-inner').innerHTML = nightInfo();
  }

  // Nudge static content a few px every minute so nothing sits still for hours (image retention)
  function pixelShift() {
    var x = Math.round(Math.random() * 8 - 4), y = Math.round(Math.random() * 8 - 4);
    App.$$('.nt-inner').forEach(function (el) { el.style.transform = 'translate(' + x + 'px,' + y + 'px)'; });
  }

  App.on('ready', function () {
    $('#ambient').innerHTML = '<div class="amb-stage"></div>';
    $('#night').innerHTML = '<div class="nt-inner"></div>';
    amb.show = new App.Slideshow($('#ambient .amb-stage'), { layout: 'cover', captions: false, interval: 14000 });
    $('#ambient').addEventListener('click', function () { App.setMode('day'); App.go(App.current === 'photos' ? 'photos' : 'home'); });
    $('#night').addEventListener('click', function () { App.setMode('day'); App.go('home'); });
  });

  App.on('mode', function (m) {
    clearInterval(shiftTimer);
    if (m === 'ambient') { amb.show.start(); }
    else amb.show.stop();
    if (m === 'night') { paint(); shiftTimer = setInterval(pixelShift, 60000); }
    App.resetIdle();
  });
  App.on('minute', paint);
})();
