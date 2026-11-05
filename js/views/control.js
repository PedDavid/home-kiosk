// Home control: status summary, scenes, rooms of tiles, climate, cameras — via Home Assistant.
// Media players have their own tab (views/media.js).
// (the Worker maps each change to an HA service call; see worker/index.js controlDevice).
(function () {
  'use strict';
  var D = window.DATA;
  var V = { el: null, pending: {}, scene: null, camTimer: null, camAge: 0 };

  function dev(id) { return D.devices.filter(function (d) { return d.id === id; })[0]; }
  function garageDev() { return D.devices.filter(function (d) { return d.type === 'garage'; })[0]; }
  // Re-read Home Assistant shortly after a change, once the device has reported its new state.
  function settle() { clearTimeout(V.settleT); V.settleT = setTimeout(function () { App.refresh('home'); }, 1500); }

  // Optimistic device update: flip immediately, send to Home Assistant, revert if that fails.
  App.setDevice = function (id, patch) {
    var d = dev(id), before = {};
    var send = App.write('home', 'POST', '/home/device/' + encodeURIComponent(id), patch);
    if (!d) return send.then(settle, function () {}); // e.g. the doorbell's lock when it has no tile
    Object.keys(patch).forEach(function (k) { before[k] = d[k]; d[k] = patch[k]; });
    V.pending[id] = true;
    changed([id]);
    return send.then(function () {
      delete V.pending[id];
      paintTile(id);
      settle();
    }, function () {
      delete V.pending[id];
      Object.keys(before).forEach(function (k) { d[k] = before[k]; });
      App.toast('Couldn’t reach <b>' + App.esc(d.name) + '</b> · it may be offline', { tone: 'error', icon: 'bolt' });
      changed([id]);
    });
  };

  // ------------------------------------------------------------ tiles
  function tile(d) {
    var html = tileHTML(d);
    return d.offline ? html.replace('class="tile ', 'class="tile is-offline ') : html;
  }
  function tileHTML(d) {
    var pend = V.pending[d.id] ? '<i class="pend"></i>' : '';
    if (d.type === 'light') {
      return '<div class="tile tile--light' + (d.on ? ' is-on' : '') + '" data-dev="' + d.id + '" style="--lvl:' + (d.on ? d.level : 0) + '%">' +
        '<span class="tile-fill"></span>' + pend +
        '<span class="tile-icon">' + App.icon('bulb', 28) + '</span>' +
        '<span class="tile-name">' + d.name + '</span>' +
        '<span class="tile-state num">' + (d.offline ? 'Offline' : d.on ? (d.dims ? d.level + '%' : 'On') : 'Off') + '</span></div>';
    }
    if (d.type === 'blind') {
      return '<div class="tile tile--blind' + (d.pos > 0 ? ' is-on' : '') + '" data-dev="' + d.id + '" style="--lvl:' + d.pos + '%">' +
        '<span class="tile-fill"></span>' + pend +
        '<span class="tile-icon">' + App.icon('blinds', 28) + '</span>' +
        '<span class="tile-name">' + d.name + '</span>' +
        '<span class="tile-state num">' + (d.pos === 0 ? 'Closed' : d.pos === 100 ? 'Open' : d.pos + '% open') + '</span></div>';
    }
    if (d.type === 'lock') {
      return '<div class="tile tile--lock' + (d.locked ? '' : ' is-warn') + '" data-dev="' + d.id + '">' + pend +
        '<span class="tile-icon">' + App.icon(d.locked ? 'lock' : 'unlock', 28) + '</span>' +
        '<span class="tile-name">' + d.name + '</span>' +
        '<span class="tile-state">' + (d.locked ? 'Locked<small>Hold to unlock</small>' : 'Unlocked<small>Tap to lock</small>') + '</span>' +
        '<svg class="hold-ring" viewBox="0 0 40 40"><circle cx="20" cy="20" r="17"/></svg></div>';
    }
    if (d.type === 'garage') {
      var label = { closed: 'Closed', open: 'Open', opening: 'Opening…', closing: 'Closing…' }[d.state];
      return '<div class="tile tile--garage' + (d.state !== 'closed' ? ' is-warn' : '') + (/ing$/.test(d.state) ? ' is-moving' : '') + '" data-dev="' + d.id + '">' + pend +
        '<span class="tile-icon">' + App.icon('garage', 28) + '</span>' +
        '<span class="tile-name">' + d.name + '</span>' +
        '<span class="tile-state">' + (d.offline ? 'Offline' : label) + '</span></div>';
    }
    return '';
  }

  function summary() {
    if (!D.homeOnline) return '<div class="hc-offline">' + App.icon('bolt', 26) + '<span><b>Can’t reach Home Assistant</b>' + App.esc(D.homeError || '') + '</span></div>';
    var lights = D.devices.filter(function (d) { return d.type === 'light' && d.on; });
    var locks = D.devices.filter(function (d) { return d.type === 'lock'; });
    var unlocked = locks.filter(function (d) { return !d.locked; });
    var garage = garageDev(), gOpen = garage && garage.state !== 'closed';
    var secure = !unlocked.length && !gOpen;
    var out = []; // climate has its own panel on the right; media has its own tab
    out.push('<button class="sum' + (lights.length ? ' sum--warm' : '') + '" data-sum="lights"><span class="label">Lights</span><b class="num">' + lights.length + '</b><small>on in ' + unique(lights.map(function (l) { return l.room; })).length + ' rooms</small></button>');
    if (locks.length || garage) out.push('<button class="sum' + (secure ? '' : ' sum--warn') + '" data-sum="security"><span class="label">Security</span><b class="num">' + (secure ? 'Locked' : unlocked.length + (gOpen ? 1 : 0)) + '</b><small>' + (secure ? (garage ? 'All doors + garage' : 'All doors') : (gOpen ? 'Garage ' + garage.state : '') + (unlocked.length ? (gOpen ? ' · ' : '') + unlocked.map(function (u) { return u.name; }).join(', ') : '') + ' open') + '</small></button>');
    var E = D.energy, T = D.energyToday;
    if (E) out.push('<button class="sum' + (E.high ? ' sum--warn' : '') + '" data-sum="energy"><span class="label">Energy</span><b class="num">' + App.fmtWatts(E.watts) + '</b><small>' +
      (T ? T.total.toFixed(1) + ' kWh today' : 'now · tap for today') + '</small></button>');
    return out.join('');
  }
  function unique(a) { return a.filter(function (x, i) { return a.indexOf(x) === i; }); }

  function scenes() {
    return D.scenes.map(function (s) {
      return '<button class="scene' + (V.scene === s.id ? ' is-on' : '') + '" data-scene="' + s.id + '">' + App.icon(s.icon, 26) + '<span>' + App.esc(s.name) + '</span></button>';
    }).join('');
  }

  function roomCount(room) {
    var on = D.devices.filter(function (d) { return d.room === room && d.type === 'light' && d.on; }).length;
    return on ? on + ' on' : 'All off';
  }
  function rooms() {
    return D.rooms.map(function (r) {
      var list = D.devices.filter(function (d) { return d.room === r.id; });
      var on = list.filter(function (d) { return d.type === 'light' && d.on; }).length;
      return '<section class="room"><div class="room-h"><h3 class="serif">' + r.name + '</h3><span class="label" data-room-count="' + r.id + '">' + roomCount(r.id) + '</span></div>' +
        '<div class="tiles">' + list.map(tile).join('') + '</div></section>';
    }).join('');
  }

  function climate() {
    var c = D.climate;
    if (!c) return '';
    // Above the set point a heat-only system (a boiler: no cool or heat_cool mode) is just idle.
    var cools = c.modes.indexOf('cool') > -1 || c.modes.indexOf('heat_cool') > -1;
    var verb = c.mode === 'off' || c.set == null ? 'Off' : c.set > c.current ? 'Heating' : c.set < c.current ? (cools ? 'Cooling' : 'Idle') : 'Holding';
    var modes = [['heat', 'Heat'], ['cool', 'Cool'], ['auto', 'Auto'], ['off', 'Off']].filter(function (m) {
      return !c.modes.length || c.modes.indexOf(m[0] === 'auto' ? 'heat_cool' : m[0]) > -1 || (m[0] === 'auto' && c.modes.indexOf('auto') > -1);
    });
    return '<div class="label label--light">Climate</div>' +
      '<div class="th-main"><button class="th-btn" data-temp="-1" aria-label="Cooler">' + App.icon('minus', 36) + '</button>' +
      '<div class="th-set"><span class="num">' + (c.set != null ? (+c.set).toFixed(1).replace('.0', '') + '°' : '–') + '</span><small>Set to</small></div>' +
      '<button class="th-btn" data-temp="1" aria-label="Warmer">' + App.icon('plus', 36) + '</button></div>' +
      '<div class="th-now"><span><b class="num">' + (c.current != null ? c.current + '°' : '–') + '</b> inside</span><span class="th-verb th-verb--' + verb.toLowerCase() + '">' + verb + '</span>' +
      (c.humidity != null ? '<span><b class="num">' + c.humidity + '%</b> humidity</span>' : '<span></span>') + '</div>' +
      '<div class="seg seg--dark">' + modes.map(function (m) {
        return '<button class="' + (c.mode === m[0] ? 'is-on' : '') + '" data-tmode="' + m[0] + '">' + m[1] + '</button>';
      }).join('') + '</div>';
  }

  function cams() {
    if (!D.cameras.length) return '';
    V.camAge = 0;
    return '<div class="label">Cameras · snapshots every 20s</div><div class="cams">' + D.cameras.slice(0, 2).map(function (c) {
      return '<button class="cam" data-cam="' + c.id + '"><img src="' + App.cameraSrc(c.id) + '" alt=""><span class="cam-name">' + App.esc(c.name) + '</span><span class="cam-age">now</span></button>';
    }).join('') + '</div>';
  }

  // Granular updates: an action repaints only what it touched (full paint() rebuilds ~270 nodes,
  // which costs ~350ms on the wall tablet).
  function paintTile(id) {
    var d = dev(id);
    [V.el && V.el.querySelector('.hc-rooms'), V.sheetBox].forEach(function (root) {
      var old = root && root.querySelector('[data-dev="' + id + '"]');
      if (!old) return;
      var tmp = document.createElement('div');
      tmp.innerHTML = tile(d);
      var nt = tmp.firstChild;
      old.parentNode.replaceChild(nt, old);
      bindTile(nt);
    });
    var c = V.el && V.el.querySelector('[data-room-count="' + d.room + '"]');
    if (c) c.textContent = roomCount(d.room);
  }
  function paintPart(sel, html) { if (V.el) V.el.querySelector(sel).innerHTML = html; }
  function changed(ids) {
    ids.forEach(paintTile);
    paintPart('.hc-sum', summary());
    App.emit('data');
  }

  function paint() {
    if (!V.el) return;
    var el = V.el;
    el.querySelector('.hc-sum').innerHTML = summary();
    el.querySelector('.hc-scenes').innerHTML = scenes();
    var rs = el.querySelector('.hc-rooms'), st = rs.scrollTop;
    rs.innerHTML = rooms();
    rs.scrollTop = st;
    bindTiles(rs);
    el.querySelector('.hc-sum').style.gridTemplateColumns = 'repeat(' + Math.max(1, el.querySelector('.hc-sum').children.length) + ',1fr)';
    el.querySelector('.hc-scenes').hidden = !D.scenes.length;
    var cl = el.querySelector('.hc-climate');
    cl.innerHTML = climate(); cl.hidden = !D.climate;
  }

  // ------------------------------------------------------------ tile gestures
  // Lights + blinds: tap toggles, horizontal drag sets level (whole tile is the slider).
  function sliderTile(t, d) {
    var sx, startLvl, dragging = false, w;
    t.addEventListener('pointerdown', function (e) {
      sx = e.clientX; dragging = false; w = t.getBoundingClientRect().width;
      startLvl = d.type === 'light' ? (d.on ? d.level : 0) : d.pos;
      try { t.setPointerCapture(e.pointerId); } catch (err) {}
    });
    t.addEventListener('pointermove', function (e) {
      if (sx == null) return;
      var dx = e.clientX - sx;
      if (!dragging && Math.abs(dx) > 10) { dragging = true; t.classList.add('is-drag'); }
      if (!dragging) return;
      var lvl = Math.max(0, Math.min(100, Math.round(startLvl + dx / w * 100)));
      t.style.setProperty('--lvl', lvl + '%');
      t.classList.toggle('is-on', lvl > 0);
      t.querySelector('.tile-state').textContent = d.type === 'light' ? (lvl ? lvl + '%' : 'Off') : (lvl === 0 ? 'Closed' : lvl === 100 ? 'Open' : lvl + '% open');
      t._lvl = lvl;
    });
    t.addEventListener('pointerup', function () {
      if (sx == null) return;
      sx = null;
      t.classList.remove('is-drag');
      V.scene = null;
      if (dragging) {
        var lvl = t._lvl;
        if (d.type === 'light') App.setDevice(d.id, lvl ? { on: true, level: lvl } : { on: false });
        else App.setDevice(d.id, { pos: lvl });
      } else if (d.type === 'light') App.setDevice(d.id, { on: !d.on, level: d.level || 100 });
      else App.setDevice(d.id, { pos: d.pos > 0 ? 0 : 100 });
    });
    t.addEventListener('pointercancel', function () { sx = null; t.classList.remove('is-drag'); paintTile(d.id); });
  }

  function bindTiles(root) { App.$$('[data-dev]', root).forEach(bindTile); }
  function bindTile(t) {
    (function () {
      var d = dev(t.dataset.dev);
      if (!d || d.offline) return;
      if (d.type === 'light' || d.type === 'blind') sliderTile(t, d);
      if (d.type === 'lock') {
        App.hold(t, 1000, {
          enabled: function () { return d.locked; },
          done: function () { V.lastHold = Date.now(); App.setDevice(d.id, { locked: false }); App.toast('<b>' + d.name + '</b> unlocked', { icon: 'unlock' }); },
          cancel: function () { App.toast('Hold for a second to unlock <b>' + d.name + '</b>', { icon: 'lock', ms: 2200 }); }
        });
        t.addEventListener('click', function () { if (!d.locked && Date.now() - (V.lastHold || 0) > 800) App.setDevice(d.id, { locked: true }); });
      }
      if (d.type === 'garage') t.addEventListener('click', function () { App.openGarageSheet(); });
    })();
  }

  // Slide-to-confirm for the garage
  App.openGarageSheet = function () {
    var g = garageDev();
    if (!g) return;
    var opening = g.state === 'closed';
    App.sheet('<div class="sheet-pad"><div class="label">Garage door · currently ' + g.state + '</div><h2 class="serif sheet-h">' + (opening ? 'Open' : 'Close') + ' the garage?</h2>' +
      '<p class="sheet-p">' + (opening ? 'Make sure the driveway is clear.' : 'The door will stop and reverse if anything is in the way.') + '</p>' +
      '<div class="slide-confirm"><span class="sc-label">Slide to ' + (opening ? 'open' : 'close') + ' →</span><span class="sc-fill"></span><span class="sc-knob">' + App.icon('garage', 30) + '</span></div></div>', {
      onMount: function (s, close) {
        var track = s.querySelector('.slide-confirm'), knob = s.querySelector('.sc-knob'), fill = s.querySelector('.sc-fill');
        var sx = null, max;
        knob.addEventListener('pointerdown', function (e) {
          sx = e.clientX; max = track.getBoundingClientRect().width - knob.getBoundingClientRect().width - 8;
          knob.setPointerCapture(e.pointerId);
        });
        knob.addEventListener('pointermove', function (e) {
          if (sx == null) return;
          var k = App.$('#stage').getBoundingClientRect().height / 800;
          var x = Math.max(0, Math.min(max / k, (e.clientX - sx) / k));
          knob.style.transform = 'translateX(' + x + 'px)';
          fill.style.width = (x + 76) + 'px';
          knob._x = x; knob._max = max / k;
        });
        knob.addEventListener('pointerup', function () {
          if (sx == null) return;
          sx = null;

          if (knob._x >= knob._max - 6) {
            close();
            V.scene = null;
            App.setDevice(g.id, { state: opening ? 'opening' : 'closing' });
            App.toast('Garage ' + (opening ? 'opening' : 'closing') + '…', { icon: 'garage', ms: 2500 });
          } else { knob.style.transform = ''; fill.style.width = ''; }
        });
      }
    });
  };

  App.cameraSheet = function (c) {
    App.sheet('<div class="cam-big"><img src="' + App.cameraSrc(c.id) + '" alt=""><div class="cam-big-bar"><span class="tk-live"><i></i>' + App.esc(c.name) + '</span><span class="label label--light">Snapshot · just now</span></div></div>', { cls: 'sheet--cam' });
  };

  function listSheet(kind) {
    var list = D.devices.filter(function (d) { return kind === 'lights' ? d.type === 'light' : (d.type === 'lock' || d.type === 'garage'); });
    if (kind === 'lights') list.sort(function (a, b) { return b.on - a.on; });
    App.sheet('<div class="sheet-pad"><div class="label">' + (kind === 'lights' ? 'Every light in the house' : 'Doors + garage') + '</div>' +
      '<h2 class="serif sheet-h">' + (kind === 'lights' ? 'Lights' : 'Security') + '</h2>' +
      '<div class="tiles tiles--sheet">' + list.map(tile).join('') + '</div>' +
      '<div class="sheet-actions">' + (kind === 'lights' ? '<button class="btn btn--solid" data-alloff>Turn everything off</button>' : '<button class="btn btn--solid" data-lockall>Lock all doors</button>') + '</div></div>', {
      cls: 'sheet--wide',
      onClose: function () { V.sheetBox = null; },
      onMount: function (s, close) {
        var box = s.querySelector('.tiles--sheet');
        V.sheetBox = box;
        bindTiles(box);
        var off = s.querySelector('[data-alloff]'), lk = s.querySelector('[data-lockall]');
        if (off) off.onclick = function () {
          var prev = D.devices.filter(function (d) { return d.type === 'light' && d.on && !d.offline; });
          close();
          prev.forEach(function (d) { App.setDevice(d.id, { on: false }); });
          App.toast('Turned off ' + prev.length + ' lights', { icon: 'bulb', undo: function () { prev.forEach(function (d) { App.setDevice(d.id, { on: true, level: d.level }); }); } });
        };
        if (lk) lk.onclick = function () {
          var locks = D.devices.filter(function (d) { return d.type === 'lock' && !d.locked && !d.offline; });
          close();
          locks.forEach(function (d) { App.setDevice(d.id, { locked: true }); });
          App.toast(locks.length ? 'Locking ' + locks.length + (locks.length === 1 ? ' door' : ' doors') : 'All doors already locked', { icon: 'lock' });
        };
      }
    });
  }

  function applyScene(id) {
    var s = D.scenes.filter(function (x) { return x.id === id; })[0];
    V.scene = id;
    paintPart('.hc-scenes', scenes());
    App.write('home', 'POST', '/home/scene', { id: id }).then(function () {
      App.toast('<b>' + App.esc(s.name) + '</b> scene on', { icon: s.icon });
      settle();
    }, function () { V.scene = null; paintPart('.hc-scenes', scenes()); });
  }

  function onClick(e) {
    var t;
    if ((t = e.target.closest('[data-scene]'))) return applyScene(t.dataset.scene);
    if ((t = e.target.closest('[data-sum]'))) {
      var k = t.dataset.sum;
      if (k === 'lights' || k === 'security') return listSheet(k);
      if (k === 'energy') return App.energySheet();
      return;
    }
    if ((t = e.target.closest('[data-temp]'))) {
      var c = D.climate, step = c.step || 0.5;
      var base = c.set != null ? +c.set : (c.current != null ? Math.round(c.current) : 20);
      c.set = Math.round(Math.max(c.min, Math.min(c.max, base + step * +t.dataset.temp)) / step) * step;
      // Taps come in bursts: send only the last target temperature.
      clearTimeout(V.tempT);
      V.tempT = setTimeout(function () { App.write('home', 'POST', '/home/climate', { set: c.set }).then(settle); }, 800);
      return climateChanged();
    }
    if ((t = e.target.closest('[data-tmode]'))) {
      D.climate.mode = t.dataset.tmode;
      App.write('home', 'POST', '/home/climate', { mode: t.dataset.tmode }).then(settle);
      return climateChanged();
    }
    if ((t = e.target.closest('[data-cam]'))) return App.cameraSheet(D.cameras.filter(function (c) { return c.id === t.dataset.cam; })[0]);
  }
  function climateChanged() { paintPart('.hc-climate', climate()); paintPart('.hc-sum', summary()); App.emit('data'); }

  // Count up since the last snapshot and fetch fresh ones every 20 s while this view is open.
  function camTimer() {
    clearInterval(V.camTimer);
    V.camTick = V.camTick || 0; // kept across re-renders, which restart this timer: resetting it here would stop the 20 s from ever arriving
    V.camTimer = setInterval(function () {
      V.camAge++;
      // Every 20 s try to refresh every picture. The label counts seconds since the last time they all
      // refreshed (and says more if the Worker had to send one it kept): a refresh that fails leaves it
      // counting, instead of saying "now" over an old picture.
      if (++V.camTick >= 20) {
        V.camTick = 0;
        var cams = App.$$('.cam', V.el), left = cams.length, worst = 0, failed = false;
        cams.forEach(function (b) {
          // done() always comes (-1 = no new picture), so a skipped, hung or failed one can't leave the
          // round open; the label only resets when every picture really refreshed.
          App.refreshCamera(b.querySelector('img'), b.dataset.cam, function (stale) {
            if (stale < 0) failed = true; else worst = Math.max(worst, stale);
            if (--left === 0 && !failed) V.camAge = worst;
          });
        });
      }
      App.$$('.cam-age', V.el).forEach(function (a) { a.textContent = V.camAge ? V.camAge + 's ago' : 'now'; });
    }, 1000);
  }

  App.register('control', {
    show: function (el) {
      if (!V.el) {
        V.el = el;
        el.innerHTML = '<div class="hc">' +
          '<div class="hc-left"><div class="hc-sum"></div><div class="hc-scenes"></div><div class="hc-rooms"></div></div>' +
          '<div class="hc-right"><div class="hc-climate"></div><div class="hc-cams"></div></div>' +
          '</div>';
        el.addEventListener('click', onClick);
      }
      var hc = el.querySelector('.hc-cams');
      App.$$('img', hc).forEach(App.releaseCamera); // the pictures being replaced: free their blobs
      hc.innerHTML = cams();
      paint();
      camTimer();
    },
    resume: function () { camTimer(); },
    hide: function () { clearInterval(V.camTimer); }
  });
})();
