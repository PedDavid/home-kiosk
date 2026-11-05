// Talks to the Worker API: first load, polling, writes, and Home Assistant alert triggers.
// Each source has its own cadence. A poll that started before a local write finished is dropped,
// so an optimistic change on screen never flickers back to the old value.
(function () {
  'use strict';
  var D = window.DATA, $ = App.$;
  var CACHE_KEY = 'hq-cache-v1';
  var PERIOD = { state: 30e3, calendar: 5 * 60e3, weather: 15 * 60e3, home: 30e3 };
  var REDRAW = { state: ['home', 'shopping'], calendar: ['home', 'calendar'], weather: ['home'], home: ['home', 'control', 'media'] };
  var last = {}, due = {}, wrote = {}, cache = {};
  var locked = false;

  // ------------------------------------------------------------ requests
  function request(method, path, data) {
    var opts = { method: method, credentials: 'same-origin', headers: {} };
    if (data instanceof FormData) opts.body = data;
    else if (data !== undefined) { opts.headers['content-type'] = 'application/json'; opts.body = JSON.stringify(data); }
    return fetch('/api' + path, opts).then(function (r) {
      if (r.status === 401) { showLocked(); throw new Error('This display is not connected'); }
      return r.json().catch(function () { return {}; }).then(function (j) {
        if (!r.ok) throw new Error(j.error || 'HTTP ' + r.status);
        return j;
      });
    });
  }
  App.api = request;
  App.uid = function () { return Date.now().toString(36) + Math.random().toString(36).slice(2, 8); };

  // A user change: save it, hold back stale polls for that source, and say so if it failed.
  App.write = function (source, method, path, data) {
    wrote[source] = Date.now();
    return request(method, path, data).then(function (r) {
      wrote[source] = Date.now();
      return r;
    }, function (err) {
      wrote[source] = Date.now();
      App.toast('Couldn’t save · ' + App.esc(err.message), { tone: 'error', icon: 'bolt' });
      refresh(source);
      throw err;
    });
  };

  // ------------------------------------------------------------ applying data
  // Keep existing objects (views and bound tiles hold references) and update them in place.
  function merge(list, fresh) {
    var by = {};
    list.forEach(function (x) { by[x.id] = x; });
    return fresh.map(function (x) {
      var old = by[x.id];
      if (!old) return x;
      Object.keys(old).forEach(function (k) { if (!(k in x)) delete old[k]; });
      return Object.assign(old, x);
    });
  }
  function localDate(s) { var a = s.split('-'); return new Date(+a[0], +a[1] - 1, +a[2]); }

  var APPLY = {
    state: function (s) {
      var tracked = {};
      D.people.forEach(function (p) { tracked[p.id] = p; });
      D.people = s.people.map(function (p) { return Object.assign(tracked[p.id] || {}, p); });
      D.ALL = D.people.map(function (p) { return p.id; });
      D.stores = s.stores; D.catOrder = s.catOrder; D.catalog = s.catalog; D.usuals = s.usuals;
      D.groceries = merge(D.groceries, s.groceries);
      D.photos = merge(D.photos, s.photos);
      D.hasHome = s.hasHome; D.scenes = s.scenes; D.cameras = s.cameras;
      D.bins = s.bins || null;
      D.notes = s.notes || [];
      D.awayOffMinutes = s.awayOffMinutes || 0;
      App.$$('.rail-item[data-go="control"], .rail-item[data-go="media"]').forEach(function (r) { r.hidden = !D.hasHome; });
    },
    calendar: function (c) {
      D.calendarErrors = c.errors || [];
      D.events = c.events.map(function (e) {
        return Object.assign({}, e, {
          start: e.allDay ? localDate(e.start) : new Date(e.start),
          end: e.allDay ? localDate(e.end) : new Date(e.end)
        });
      });
    },
    weather: function (w) {
      D.weather = w && !w.none && !w.error ? w : null;
      D.warnings = (w && w.warnings) || [];
    },
    home: function (h, quiet) {
      if (h.none) { D.hasHome = false; return; }
      D.homeOnline = !!h.online;
      D.homeError = h.error || null;
      if (!h.online) return;
      var prev = D.sensors || {};
      D.rooms = h.rooms; D.devices = merge(D.devices, h.devices);
      D.climate = h.climate; D.cameras = h.cameras; D.scenes = h.scenes;
      D.alertConfig = h.alertConfig;
      D.homeVia = h.via || null; D.homeViaAt = h.viaAt || 0;
      D.energy = h.energy || null;
      D.checks = h.checks || null;
      D.players = h.players || [];
      h.presence.forEach(function (x) {
        var p = D.people.filter(function (q) { return q.id === x.id; })[0];
        if (p) { p.home = x.home; p.status = x.status; }
      });
      D.sensors = h.sensors;
      if (!quiet && cache.home) alerts(prev, h.sensors); // only on live changes, never on boot
    }
  };

  function apply(source, data, quiet) {
    var text = JSON.stringify(data);
    if (text === last[source]) return false;
    last[source] = text;
    APPLY[source](data, quiet);
    cache[source] = data;
    try { localStorage.setItem(CACHE_KEY, JSON.stringify(cache)); } catch (e) {}
    if (!quiet) changed(source);
    return true;
  }

  // Redraw what changed, unless someone is mid-tap (they'll get it on the next poll).
  function changed(source) {
    App.emit('data');
    var busy = App.idleFor() < 6 || $('#sheetLayer').classList.contains('is-open') || document.activeElement && document.activeElement.tagName === 'INPUT';
    if (!busy && App.mode === 'day' && REDRAW[source].indexOf(App.current) > -1) App.rerender();
  }

  function refresh(source) {
    if (source === 'home' && !D.hasHome) return Promise.resolve();
    var started = Date.now();
    due[source] = started + PERIOD[source];
    return request('GET', '/' + (source === 'state' ? 'state' : source)).then(function (data) {
      if (wrote[source] && wrote[source] >= started) return; // a local write raced this poll
      apply(source, data);
    }, function (err) {
      if (source === 'home') { D.homeOnline = false; D.homeError = err.message; }
      console.warn('[sync] ' + source + ': ' + err.message);
    });
  }
  App.refresh = refresh;

  // ------------------------------------------------------------ alerts from Home Assistant
  var garageAlerted = null;
  function alerts(prev, now) {
    var cfg = D.alertConfig || {};
    if (cfg.doorbell && now.doorbell && prev.doorbell && now.doorbell.changed !== prev.doorbell.changed &&
      ['unavailable', 'unknown', 'off'].indexOf(now.doorbell.state) < 0) App.doorbell();
    if (now.leak && !prev.leak) App.leak();
    // Smoke/CO: take over when a detector goes off that wasn't already going off.
    var was = (prev.smoke || []).map(function (s) { return s.name; });
    if ((now.smoke || []).some(function (s) { return was.indexOf(s.name) < 0; })) App.smoke(now.smoke);
    if (cfg.garageOpenMinutes && now.garageOpenSince && garageAlerted !== now.garageOpenSince) {
      var mins = (Date.now() - Date.parse(now.garageOpenSince)) / 60000;
      if (mins >= cfg.garageOpenMinutes) { garageAlerted = now.garageOpenSince; App.garageOpenAlert(Math.round(mins)); }
    }
  }

  // ------------------------------------------------------------ not connected
  function showLocked() {
    if (locked) return;
    locked = true;
    var show = function () {
      var el = $('#takeover');
      el.className = 'takeover takeover--alarm is-open';
      el.innerHTML = '<div class="tk-alarm"><div class="tk-alarm-icon">' + App.icon('lock', 88) + '</div>' +
        '<div class="label label--light">Home HQ</div><h1 class="tk-huge">Not connected</h1>' +
        '<p class="tk-sub">Open this page once with <b>?key=</b> and your dashboard key on the end of the address.</p></div>';
    };
    if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', show); else show();
  }

  // ------------------------------------------------------------ boot + polling
  try {
    var saved = JSON.parse(localStorage.getItem(CACHE_KEY) || 'null');
    if (saved) Object.keys(APPLY).forEach(function (k) { if (saved[k]) apply(k, saved[k], true); });
  } catch (e) {}
  var hadCache = !!cache.state;

  var first = request('GET', '/state').then(function (s) {
    apply('state', s, true);
    var started = Date.now();
    ['calendar', 'weather', 'home'].forEach(function (k) { due[k] = started + PERIOD[k]; });
    due.state = started + PERIOD.state;
    return Promise.all(['calendar', 'weather', 'home'].map(function (k) {
      if (k === 'home' && !D.hasHome) return null;
      return request('GET', '/' + k).then(function (d) { apply(k, d, true); }, function (err) { console.warn('[sync] ' + k + ': ' + err.message); });
    }));
  }).catch(function (err) { console.warn('[sync] first load: ' + err.message); });

  // With a saved copy, start right away and redraw when fresh data lands; without one, wait (max 8 s).
  window.DATA_READY = hadCache ? Promise.resolve() : Promise.race([first, new Promise(function (r) { setTimeout(r, 8000); })]);
  if (hadCache) first.then(function () { if (App.current) { App.emit('data'); App.rerender(); } });

  App.on('ready', function () {
    setInterval(function () {
      if (locked) return;
      var now = Date.now();
      Object.keys(PERIOD).forEach(function (k) {
        var period = k === 'home' && App.mode !== 'day' ? 60e3 : PERIOD[k];
        if (!due[k] || now >= due[k] - PERIOD[k] + period) refresh(k);
      });
    }, 5000);
    window.addEventListener('online', function () { Object.keys(PERIOD).forEach(refresh); });
    // Pushed alerts: Home Assistant bumps a counter on the Worker the moment the doorbell, a smoke
    // alarm or a leak sensor changes, and the Worker sends it down this WebSocket (in every mode: the
    // Home poll slows to 60 s on the photo screen and at night). When the counter moves, refresh Home
    // now and the alerts fire. Nothing polls while it's open. A "ping" every 30 s keeps the
    // connection honest: no "pong" for 75 s means it's dead (Wi-Fi dropped) and it reconnects. While
    // it's down, ask for the counter every 10 s instead, so an alert is never missed.
    var alertSeen = null, sock = null, sockOpen = false, backoff = 2000, lastPong = 0;
    function onAlert(a) {
      if (alertSeen !== null && a.seq !== alertSeen) refresh('home');
      alertSeen = a.seq;
    }
    function reconnect() {
      sockOpen = false; sock = null;
      setTimeout(connect, backoff);
      backoff = Math.min(backoff * 2, 30000);
    }
    function connect() {
      if (locked || !D.hasHome || !window.WebSocket) { setTimeout(connect, 5000); return; }
      var ws;
      try { ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/api/alerts/ws'); } catch (e) { reconnect(); return; }
      sock = ws;
      ws.onopen = function () { sockOpen = true; backoff = 2000; lastPong = Date.now(); };
      ws.onmessage = function (e) {
        if (e.data === 'pong') { lastPong = Date.now(); return; }
        try { onAlert(JSON.parse(e.data)); } catch (err) {}
      };
      ws.onclose = function () { if (sock === ws) reconnect(); };
      ws.onerror = function () { try { ws.close(); } catch (e) {} };
    }
    setInterval(function () {
      if (sockOpen && sock) {
        if (Date.now() - lastPong > 75000) {
          // No pong: the connection is dead (Wi-Fi dropped silently). Don't wait for the close handshake,
          // which can take a long time on a half-open connection and meanwhile keeps the fallback poll off:
          // forget the socket (its onclose then does nothing) and reconnect now.
          var dead = sock;
          try { dead.close(); } catch (e) {}
          reconnect();
          return;
        }
        try { sock.send('ping'); } catch (e) {}
      }
    }, 30000);
    setInterval(function () {
      if (sockOpen || locked || !D.hasHome) return;
      request('GET', '/alerts').then(onAlert, function () {});
    }, 10000);
    connect();
  });
})();
