// Core shell: router, clock, idle/ambient, toasts, sheets, helpers.
(function () {
  'use strict';
  var D = window.DATA;
  var App = window.App = { state: D, views: {}, current: null, mode: 'day' };
  var $ = App.$ = function (s, r) { return (r || document).querySelector(s); };
  App.$$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };

  // ------------------------------------------------------------ events
  var handlers = {};
  App.on = function (e, fn) { (handlers[e] = handlers[e] || []).push(fn); };
  App.emit = function (e, a) { (handlers[e] || []).forEach(function (fn) { fn(a); }); };

  // ------------------------------------------------------------ time (always the real clock)
  App.now = function () { return new Date(); };
  App.dayStart = function (d) { var x = new Date(d); x.setHours(0, 0, 0, 0); return x; };
  App.addDays = function (d, n) { var x = new Date(d); x.setDate(x.getDate() + n); return x; };
  App.sameDay = function (a, b) { return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate(); };
  // Night and screen-off hours are per frame (display controls), kept in localStorage as "HH:MM".
  // Night: the photo screen becomes a dim clock and chimes stay quiet (10 PM to 6 AM until set).
  // Screen off: the backlight goes right down (off until switched on; 1 AM to 5 AM suggested).
  // A window may cross midnight; from = to means never.
  var NIGHT_DEFAULT = { from: '22:00', to: '06:00', offOn: false, offFrom: '01:00', offTo: '05:00' };
  function validTime(t) { return typeof t === 'string' && /^([01]\d|2[0-3]):[0-5]\d$/.test(t); }
  App.night = (function () {
    var n = Object.assign({}, NIGHT_DEFAULT);
    try {
      var v = JSON.parse(localStorage.getItem('hq-night')) || {};
      ['from', 'to', 'offFrom', 'offTo'].forEach(function (k) { if (validTime(v[k])) n[k] = v[k]; });
      n.offOn = v.offOn === true;
    } catch (e) {}
    return n;
  })();
  App.setNight = function (patch) {
    var n = Object.assign({}, App.night, patch);
    if (!['from', 'to', 'offFrom', 'offTo'].every(function (k) { return validTime(n[k]); })) return false;
    App.night = n;
    try { localStorage.setItem('hq-night', JSON.stringify(n)); } catch (e) {}
    App.syncNightMode();
    return true;
  };
  function mins(t) { return +t.slice(0, 2) * 60 + +t.slice(3); }
  function within(from, to) {
    var d = App.now(), m = d.getHours() * 60 + d.getMinutes(), a = mins(from), b = mins(to);
    return a === b ? false : a < b ? m >= a && m < b : m >= a || m < b;
  }
  App.isNight = function () { return within(App.night.from, App.night.to); };
  App.isScheduledOff = function () { return App.night.offOn && within(App.night.offFrom, App.night.offTo); };
  // "10 PM", "6:30 AM" for the notes in the display controls.
  App.nightLabel = function (t) {
    var h = +t.slice(0, 2), m = t.slice(3);
    return (h % 12 || 12) + (m === '00' ? '' : ':' + m) + ' ' + (h < 12 ? 'AM' : 'PM');
  };
  // The clock picks the resting screen at three moments only: when the idle timer runs out, at the
  // Dim, Bright, Sleep and Wake times for a frame that's resting untouched, and nowhere else.
  //   Sleep to Wake (if switched on) -> screen off; Dim to Bright -> the dim clock; else photos.
  // A frame someone is using (day) isn't interrupted: it settles into the resting screen once
  // idle. A mode picked in the display controls (manual) stays until the next tap.
  App.scheduledOff = false;
  App.restMode = function () { return App.isScheduledOff() ? 'off' : App.isNight() ? 'night' : 'ambient'; };
  // The whole UI is dimmed and amber-tinted (css: data-night) for as long as it's night, whatever
  // the mode, so a tap in the night doesn't flash a bright screen. Outside the hours it's normal.
  App.applyNight = function () { var st = $('#stage'); if (st) st.dataset.night = App.isNight() ? '1' : '0'; };
  App.rest = function () {
    var m = App.restMode();
    App.closeSheet();
    App.go('home');
    App.setMode(m);
    App.scheduledOff = m === 'off';
  };
  App.syncNightMode = function () {
    App.applyNight();
    if (App.manualMode) return;
    var m = App.mode, t = App.restMode();
    if (!(m === 'ambient' || m === 'night' || (m === 'off' && App.scheduledOff)) || t === m) return;
    if (t === 'off' && $('#takeover').classList.contains('is-open')) return;
    App.closeSheet();
    App.setMode(t);
    App.scheduledOff = t === 'off';
  };

  var DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
  var MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
  var MON_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'June', 'July', 'Aug', 'Sept', 'Oct', 'Nov', 'Dec'];
  App.fmt = {
    time: function (d, ampm) {
      var h = d.getHours(), m = d.getMinutes();
      var s = (h % 12 || 12) + ':' + (m < 10 ? '0' : '') + m;
      return ampm === false ? s : s + ' ' + (h >= 12 ? 'PM' : 'AM');
    },
    // "4 PM", "3:45 PM"
    hour: function (d) {
      var h = d.getHours(), m = d.getMinutes();
      return (h % 12 || 12) + (m ? ':' + (m < 10 ? '0' : '') + m : '') + ' ' + (h >= 12 ? 'PM' : 'AM');
    },
    ampm: function (d) { return d.getHours() >= 12 ? 'PM' : 'AM'; },
    range: function (a, b) {
      var same = App.fmt.ampm(a) === App.fmt.ampm(b);
      return (same ? App.fmt.time(a, false) : App.fmt.time(a)) + ' – ' + App.fmt.time(b);
    },
    day: function (d) { return DAYS[d.getDay()]; },
    dayShort: function (d) { return DAYS[d.getDay()].slice(0, 3); },
    month: function (d) { return MONTHS[d.getMonth()]; },
    monthShort: function (d) { return MON_SHORT[d.getMonth()]; },
    mins: function (m) {
      m = Math.max(0, Math.round(m));
      if (m < 60) return m + ' min';
      var h = Math.floor(m / 60), r = m % 60;
      return h + ' hr' + (r ? ' ' + r + ' min' : '');
    },
    rel: function (d) {
      var t = App.dayStart(App.now()), x = App.dayStart(d), diff = Math.round((x - t) / 864e5);
      if (diff === 0) return 'Today';
      if (diff === 1) return 'Tomorrow';
      if (diff === -1) return 'Yesterday';
      return DAYS[d.getDay()];
    }
  };
  App.esc = function (s) { return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; }); };

  // ------------------------------------------------------------ icons + people
  App.icon = function (name, size, cls) {
    size = size || 24;
    return '<svg class="i ' + (cls || '') + '" width="' + size + '" height="' + size + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + (window.ICONS[name] || '') + '</svg>';
  };
  App.person = function (id) {
    if (id === 'family' || id === 'any') return D.family;
    for (var i = 0; i < D.people.length; i++) if (D.people[i].id === id) return D.people[i];
    return D.family;
  };
  App.pstyle = function (p) {
    if (typeof p === 'string') p = App.person(p);
    return '--c:' + p.color + ';--c-rgb:' + p.rgb + ';--c-ink:' + p.ink;
  };
  App.avatar = function (p, size, cls) {
    if (typeof p === 'string') p = App.person(p);
    size = size || 48;
    return '<span class="avatar ' + (cls || '') + '" style="' + App.pstyle(p) + ';width:' + size + 'px;height:' + size + 'px;font-size:' + Math.round(size * 0.46) + 'px">' + (p.id === 'family' ? '★' : p.name[0]) + '</span>';
  };
  App.dots = function (ids, size) {
    return '<span class="dots">' + ids.map(function (id) { return '<i style="' + App.pstyle(id) + (size ? ';width:' + size + 'px;height:' + size + 'px' : '') + '"></i>'; }).join('') + '</span>';
  };

  // ------------------------------------------------------------ calendar helpers
  App.events = function () { return D.events; };
  App.isFamily = function (e) { return D.people.length > 1 && e.people.length >= D.people.length; };
  App.evColor = function (e) { return App.isFamily(e) ? D.family : App.person(e.people[0]); };
  App.timedOn = function (day, pid) {
    var s = App.dayStart(day), e = App.addDays(s, 1);
    return App.events().filter(function (ev) {
      return !ev.allDay && ev.start < e && ev.end > s && (!pid || ev.people.indexOf(pid) > -1);
    }).sort(function (a, b) { return a.start - b.start || b.end - a.end; });
  };
  App.allDayOn = function (day, pid) {
    var s = App.dayStart(day), e = App.addDays(s, 1);
    return App.events().filter(function (ev) {
      return ev.allDay && ev.start < e && ev.end > s && (!pid || ev.people.indexOf(pid) > -1);
    });
  };
  // How long it takes to get there: the live drive time (Google, from where Home Assistant places
  // the person) plus Walk:, or else the event's Travel:. Null when there's neither.
  App.journey = function (e) {
    if (e.drive) return { mins: e.drive.mins + (e.walk || 0), drive: e.drive.mins, walk: e.walk || 0, from: e.drive.from, live: true };
    return e.travel ? { mins: e.travel, live: false } : null;
  };
  App.journeyText = function (j) {
    if (!j) return '';
    return j.live ? j.drive + ' min drive' + (j.walk ? ' + ' + j.walk + ' min walk' : '') : j.mins + ' min to get there';
  };
  App.leaveBy = function (e) { var j = App.journey(e); return j ? new Date(+e.start - j.mins * 60000) : e.start; };
  // The next thing someone needs to act on: earliest upcoming "leave by" today.
  App.nextEvent = function () {
    var now = App.now();
    var list = App.timedOn(now).filter(function (e) { return e.start > now; });
    list.sort(function (a, b) { return App.leaveBy(a) - App.leaveBy(b); });
    return list[0] || null;
  };
  App.findEvent = function (id) {
    return App.events().filter(function (e) { return e.id === id; })[0];
  };

  // ------------------------------------------------------------ weather helpers
  // Forecast hours start at local midnight on W.day and run for 48 hours.
  function wxHour() {
    var W = D.weather, a = W.day.split('-');
    return Math.floor((App.now() - new Date(+a[0], +a[1] - 1, +a[2])) / 36e5);
  }
  var WX_LABEL = { sun: 'Sunny', moon: 'Clear', partly: 'Partly cloudy', cloud: 'Overcast', rain: 'Rain', snow: 'Snow' };
  App.weatherNow = function () {
    var W = D.weather;
    if (!W) return null;
    var h = Math.max(0, Math.min(W.t.length - 1, wxHour()));
    var endOfDay = Math.min(W.t.length, h - App.now().getHours() + 24);
    var wetAt = -1;
    for (var i = h; i < endOfDay; i++) if (W.p[i] >= 50) { wetAt = i; break; }
    var snow = wetAt > -1 && W.c[wetAt] === 'snow';
    var summary, advice;
    if (wetAt === h) {
      var stop = h; while (stop < W.t.length && W.p[stop] >= 50) stop++;
      summary = (snow ? 'Snow' : 'Rain') + ' until ' + App.fmt.hour(new Date(2000, 0, 1, stop % 24));
      advice = snow ? 'Boots and gloves' : 'Umbrella weather';
    } else if (wetAt > -1) {
      summary = (snow ? 'Snow' : 'Rain') + ' from ' + App.fmt.hour(new Date(2000, 0, 1, wetAt % 24));
      advice = snow ? 'Wrap up warm' : 'Grab a rain jacket';
    } else {
      var cool = W.t[h] < (W.units === 'fahrenheit' ? 54 : 12);
      summary = cool ? 'Dry and cool' : 'Dry all day';
      advice = cool ? 'Jacket weather' : 'Light layers';
    }
    var temp = h === wxHour() && W.now != null ? W.now : W.t[h];
    return { temp: temp, cond: W.c[h], label: WX_LABEL[W.c[h]], hi: W.hi, lo: W.lo, summary: summary, advice: advice, hour: h };
  };
  App.hourly = function (count) {
    var W = D.weather, out = [];
    if (!W) return out;
    var h = Math.max(0, wxHour());
    for (var i = 0; i < count && h + i < W.t.length; i++) {
      var x = (h + i) % 24;
      out.push({ h: x, t: W.t[h + i], c: W.c[h + i], p: W.p[h + i], label: i === 0 ? 'Now' : ((x % 12 || 12) + (x >= 12 ? 'p' : 'a')) });
    }
    return out;
  };

  // ------------------------------------------------------------ router
  App.register = function (name, view) { App.views[name] = view; };
  // Views are built once and kept laid out (hidden with content-visibility), so switching tabs
  // only has to paint. A view is rebuilt when its data changed ('data'/'time' events) or, for
  // time-sensitive views (minutely: true), when the minute has rolled over since it was built.
  function minuteKey() { return Math.floor(App.now().getTime() / 60000); }
  function build(name, opts) {
    var view = App.views[name];
    view.show($('#view-' + name), opts || {});
    view._built = true; view._dirty = false; view._min = minuteKey();
  }
  App.go = function (name, opts) {
    opts = opts || {};
    if (!App.views[name]) name = 'home';
    if (App.current && App.current !== name && App.views[App.current].hide) App.views[App.current].hide();
    App.$$('.view').forEach(function (v) { v.classList.toggle('is-active', v.dataset.view === name); });
    App.$$('.rail-item').forEach(function (b) { b.classList.toggle('is-active', b.dataset.go === name); });
    App.current = name;
    var view = App.views[name];
    if (!view._built || view._dirty || opts.rerender || (view.minutely && view._min !== minuteKey())) build(name, opts);
    else if (view.resume) view.resume($('#view-' + name));
    if (location.hash !== '#' + name) history.replaceState(null, '', '#' + name);
    App.emit('view', name);
  };
  // Build a hidden view and lay it out now (cached by content-visibility; no paint happens).
  function prebuild(name) {
    var el = $('#view-' + name), view = App.views[name];
    build(name);
    if (view.hide) view.hide();
    el.classList.add('is-warming');
    el.offsetHeight;
    el.classList.remove('is-warming');
  }
  // Pre-build (startup) and re-build (after data changes) hidden views in the background, one per
  // slot, so navigating to them never pays the rebuild cost.
  var refreshTimer = null;
  function refreshStale(delay) {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(function next() {
      var name = Object.keys(App.views).filter(function (k) {
        var v = App.views[k];
        return k !== App.current && (!v._built || v._dirty);
      })[0];
      if (!name) return;
      prebuild(name);
      refreshTimer = setTimeout(next, 400);
    }, delay);
  }
  function markStale() {
    Object.keys(App.views).forEach(function (k) { if (k !== App.current) App.views[k]._dirty = true; });
    refreshStale(1500);
  }
  App.on('data', markStale);
  App.on('time', markStale);
  App.rerender = function () {
    if (App.current) App.views[App.current].show($('#view-' + App.current), { rerender: true });
  };

  // ------------------------------------------------------------ toasts (single, with undo)
  var toastTimer;
  App.toast = function (html, opts) {
    opts = opts || {};
    var box = $('#toasts');
    box.innerHTML = '';
    clearTimeout(toastTimer);
    var ms = opts.ms || 5000;
    var t = document.createElement('div');
    t.className = 'toast' + (opts.tone ? ' toast--' + opts.tone : '');
    t.style.setProperty('--ms', ms + 'ms');
    t.innerHTML = (opts.icon ? App.icon(opts.icon, 26) : '') + '<span class="toast-msg">' + html + '</span>' +
      (opts.undo ? '<button class="toast-undo">' + App.icon('undo', 22) + 'Undo</button>' : '');
    box.appendChild(t);
    t.classList.add('is-in');
    function kill() { t.remove(); }
    toastTimer = setTimeout(kill, ms);
    if (opts.undo) t.querySelector('.toast-undo').addEventListener('click', function () { clearTimeout(toastTimer); opts.undo(); kill(); });
  };

  // ------------------------------------------------------------ sheets (modal cards)
  App.sheet = function (html, opts) {
    opts = opts || {};
    var layer = $('#sheetLayer');
    layer.innerHTML = '<div class="sheet-scrim"></div><div class="sheet ' + (opts.cls || '') + '">' +
      '<button class="sheet-x" data-close aria-label="Close">' + App.icon('x', 28) + '</button>' + html + '</div>';
    layer.classList.add('is-open');
    function close() {
      if (!layer.classList.contains('is-open')) return;
      layer.classList.remove('is-open');
      if (opts.onClose) opts.onClose();
      layer.innerHTML = '';
    }
    App.closeSheet = close;
    layer.querySelector('.sheet-scrim').addEventListener('click', close);
    App.$$('[data-close]', layer).forEach(function (b) { b.addEventListener('click', close); });
    if (opts.onMount) opts.onMount(layer.querySelector('.sheet'), close);
    return close;
  };
  App.closeSheet = function () {};

  // Stage-local coordinates for an element's center (stage may be scaled on desktop)
  App.centerOf = function (el) {
    var stage = $('#stage').getBoundingClientRect(), r = el.getBoundingClientRect(), k = stage.height / 800;
    return { x: (r.left + r.width / 2 - stage.left) / k, y: (r.top + r.height / 2 - stage.top) / k };
  };

  // ------------------------------------------------------------ press-and-hold
  App.hold = function (el, ms, cb) {
    var t = null;
    el.style.setProperty('--hold', ms + 'ms');
    el.addEventListener('pointerdown', function () {
      if (cb.enabled && !cb.enabled()) return;
      el.classList.add('is-holding');
      t = setTimeout(function () { t = null; el.classList.remove('is-holding'); cb.done(); }, ms);
    });
    function cancel(e) {
      if (!t) return;
      clearTimeout(t); t = null;
      el.classList.remove('is-holding');
      if (cb.cancel && e.type === 'pointerup') cb.cancel();
    }
    el.addEventListener('pointerup', cancel);
    el.addEventListener('pointerleave', cancel);
    el.addEventListener('pointercancel', cancel);
  };

  // ------------------------------------------------------------ modes: day / ambient / night
  // manual: picked from the display controls or keyboard as a preview, so the clock doesn't undo it.
  App.manualMode = false;
  App.setMode = function (m, manual) {
    var stage = $('#stage');
    App.mode = m;
    App.manualMode = !!manual;
    if (m !== 'off') App.scheduledOff = false;
    stage.dataset.mode = m;
    App.applyNight();
    App.emit('mode', m);
  };

  // ------------------------------------------------------------ idle
  // Timeouts (seconds since the last touch), per frame (display controls): back to Today, rest
  // (photo screen, dim clock or screen off), and the same during the night hours ("peek"), so a
  // look at the time in bed doesn't keep the screen up for two minutes.
  App.idle = (function () {
    var d = { home: 60, rest: 120, peek: 20 };
    try {
      var v = JSON.parse(localStorage.getItem('hq-idle')) || {};
      Object.keys(d).forEach(function (k) { if (v[k] >= 5 && v[k] <= 3600) d[k] = Math.round(v[k]); });
    } catch (e) {}
    return d;
  })();
  App.setIdle = function (patch) {
    var n = Object.assign({}, App.idle);
    Object.keys(patch).forEach(function (k) {
      var v = Math.round(+patch[k]);
      if (k in n && v >= 5 && v <= 3600) n[k] = v;
    });
    Object.assign(App.idle, n);
    try { localStorage.setItem('hq-idle', JSON.stringify(App.idle)); } catch (e) {}
  };
  var idleSec = 0;
  function onIdleTick() {
    idleSec++;
    var blocked = $('#takeover').classList.contains('is-open');
    if (App.mode !== 'day' || blocked) return;
    if (idleSec === App.idle.home && App.current !== 'home') { App.closeSheet(); App.go('home'); }
    if (idleSec >= (App.isNight() ? App.idle.peek : App.idle.rest)) App.rest();
  }
  App.resetIdle = function () { idleSec = 0; };
  App.idleFor = function () { return idleSec; };

  // ------------------------------------------------------------ stage scaling
  // Designed at 1280×800. Screens wider than 16:10 (e.g. 1920×1080 frames) widen the stage to their
  // aspect instead of pillarboxing; narrower screens letterbox as before.
  var STAGE_H = 800, STAGE_MIN_W = 1280;
  function fit() {
    var stage = $('#stage');
    var w = Math.max(STAGE_MIN_W, Math.round(STAGE_H * window.innerWidth / window.innerHeight));
    stage.style.setProperty('--stage-w', w + 'px');
    var s = Math.min(window.innerWidth / w, window.innerHeight / STAGE_H);
    stage.style.transform = Math.abs(s - 1) < 0.01 ? '' : 'scale(' + s + ')';
  }

  // ------------------------------------------------------------ clock tick
  var lastMin = -1;
  function tick() {
    var n = App.now();
    App.emit('second', n);
    if (n.getMinutes() !== lastMin) {
      lastMin = n.getMinutes();
      $('#railClock').innerHTML = '<b>' + App.fmt.time(n, false) + '</b><span>' + App.fmt.ampm(n) + '</span>';
      App.emit('minute', n);
      App.syncNightMode();
    }
    onIdleTick();
  }

  // ------------------------------------------------------------ boot
  App.start = function () {
    App.applyNight();
    fit();
    window.addEventListener('resize', fit);
    // Switch tabs the instant a finger lands (pointerdown), not on release. Keyboard "clicks"
    // (detail === 0) still work.
    $('#rail').addEventListener('pointerdown', function (e) {
      var b = e.target.closest('[data-go]');
      if (b && e.button === 0) App.go(b.dataset.go);
    });
    $('#rail').addEventListener('click', function (e) {
      var b = e.target.closest('[data-go]');
      if (b && e.detail === 0) App.go(b.dataset.go);
    });
    document.addEventListener('pointerdown', App.resetIdle, true);
    window.addEventListener('hashchange', function () { App.go(location.hash.slice(1)); });
    App.go(location.hash.slice(1) || 'home');
    tick();
    setInterval(tick, 1000);
    App.emit('ready');
    refreshStale(1500);
  };
  // js/sync.js sets DATA_READY: immediately when this device has a saved copy, else after first load.
  document.addEventListener('DOMContentLoaded', function () { (window.DATA_READY || Promise.resolve()).then(App.start); });
})();
