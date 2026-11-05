// Alerts (takeovers, banners) raised by Home Assistant via js/sync.js, + the display-controls drawer.
(function () {
  'use strict';
  var D = window.DATA, $ = App.$;

  // Refresh a camera picture in place: fetch the new one, check it loads, and only then swap it in, so
  // a slow or failed refresh (Home Assistant answers a sleeping camera with a 500 after 10 s) or a bad
  // picture leaves the old one up instead of a broken-image icon. done(seconds) is called once per call:
  // with how old the new picture is when one was swapped in (0 = fresh; the Worker sends the last good one
  // when Home Assistant fails), or -1 when nothing was (failed, timed out, skipped as busy). One refresh at a time per picture: a request can take up to 8 s while the doorbell
  // asks every 2 s, and an older answer must never land on top of a newer one.
  App.refreshCamera = function (img, id, done) {
    if (!window.fetch || !window.URL || !URL.createObjectURL) { img.src = App.cameraSrc(id); if (done) done(0); return; }
    if (img._busy) { if (done) done(-1); return; }
    img._busy = true;
    var url = null, over = false, ctl = window.AbortController ? new AbortController() : null;
    // A request that never answers (Wi-Fi dropped half-way) must not leave the picture stuck: give up
    // after 12 s (the Worker's own limit is 8 s) and let the next round try again.
    var giveUp = setTimeout(function () { if (ctl) ctl.abort(); over = true; finish(false); }, 12e3);
    var settled = false;
    function finish(keep) {
      if (settled) return;
      settled = true;
      clearTimeout(giveUp);
      if (url && !keep) URL.revokeObjectURL(url);
      img._busy = false;
      if (!keep && done) done(-1);
    }
    fetch(App.cameraSrc(id), { credentials: 'same-origin', signal: ctl ? ctl.signal : undefined }).then(function (r) {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      var stale = +r.headers.get('x-stale') || 0;
      return r.blob().then(function (b) { return { b: b, stale: stale }; });
    }).then(function (x) {
      if (over) return;
      if (!img.isConnected) { finish(false); return; }
      url = URL.createObjectURL(x.b);
      var probe = new Image();
      probe.onload = function () {
        if (over) { URL.revokeObjectURL(url); return; }
        if (!img.isConnected) { finish(false); return; }
        var before = img.getAttribute('data-blob');
        img.setAttribute('data-blob', url);
        img.src = url;
        if (before) URL.revokeObjectURL(before);
        finish(true);
        if (done) done(x.stale);
      };
      probe.onerror = function () { if (!over) finish(false); };
      probe.src = url;
    }, function () { if (!over) finish(false); });
  };
  // Let go of a picture's memory (a takeover closing: the picture is never shown again).
  App.releaseCamera = function (img) {
    var u = img && img.getAttribute('data-blob');
    if (u) { URL.revokeObjectURL(u); img.removeAttribute('data-blob'); }
  };
  App.cameraSrc = function (id) { return '/api/ha/camera?id=' + encodeURIComponent(id) + '&t=' + Date.now(); };

  // ------------------------------------------------------------ full-screen takeovers
  function takeover(html, cls) {
    var el = $('#takeover');
    clearInterval(camTimer);
    App.releaseCamera(el.querySelector('.tk-cam img')); // a takeover replaced by another one: free the old picture
    el.className = 'takeover ' + (cls || '');
    el.innerHTML = html;
    el.classList.add('is-open');
    App.closeSheet();
    if (App.mode !== 'day') App.setMode('day');
    return el;
  }
  var camTimer;
  function closeTakeover() {
    clearInterval(camTimer);
    App.stopAlarm();
    var el = $('#takeover');
    App.releaseCamera(el.querySelector('.tk-cam img'));
    el.classList.remove('is-open');
    el.innerHTML = '';
  }
  App.closeTakeover = closeTakeover;

  var doorbellTimer;
  App.doorbell = function (test) {
    var now = App.now(), cfg = D.alertConfig || {};
    var el = takeover(
      (cfg.doorbellCamera ? '<div class="tk-cam"><img src="' + App.cameraSrc(cfg.doorbellCamera) + '" alt=""><span class="tk-live"><i></i>Front door</span></div>' : '') +
      '<div class="tk-door">' +
      '  <div class="label label--light">' + App.icon('bell', 20) + ' Doorbell · ' + App.fmt.time(now) + '</div>' +
      '  <h1 class="serif tk-title">Someone’s at<br>the front door</h1>' +
      '  <div class="tk-actions">' +
      (cfg.doorbellLock ? '    <button class="tk-btn tk-btn--hold" data-unlock>' + App.icon('unlock', 30) + '<span>Hold to unlock</span></button>' : '') +
      '    <button class="tk-btn tk-btn--solid" data-dismiss>' + App.icon('x', 30) + '<span>Dismiss</span></button>' +
      '  </div>' +
      '</div>', 'takeover--door');
    el.querySelector('[data-dismiss]').onclick = function () { clearTimeout(doorbellTimer); closeTakeover(); };
    var u = el.querySelector('[data-unlock]');
    if (u) App.hold(u, 1000, {
      done: function () {
        App.setDevice(cfg.doorbellLock, { locked: false });
        u.classList.add('is-on');
        u.querySelector('span').textContent = 'Unlocked';
        App.toast('Front door unlocked', { icon: 'unlock' });
      },
      cancel: function () { App.toast('Keep holding to unlock', { ms: 2000, icon: 'lock' }); }
    });
    clearTimeout(doorbellTimer);
    doorbellTimer = setTimeout(closeTakeover, 45000);
    App.sound('doorbell', test === true);
    // Near-live: a fresh snapshot every 2 s, swapped in once loaded so the picture never blanks.
    var img = el.querySelector('.tk-cam img');
    clearInterval(camTimer);
    if (img) camTimer = setInterval(function () {
      if (!img.isConnected) return clearInterval(camTimer);
      App.refreshCamera(img, cfg.doorbellCamera);
    }, 2000);
  };

  App.leak = function () {
    var now = App.now();
    var el = takeover(
      '<div class="tk-alarm">' +
      '  <div class="tk-alarm-icon">' + App.icon('drop', 88) + '</div>' +
      '  <div class="label label--light">Critical alert · ' + App.fmt.time(now) + '</div>' +
      '  <h1 class="tk-huge">Water leak</h1>' +
      '  <p class="tk-sub">A leak sensor detected water.</p>' +
      '  <div class="tk-actions">' +
      ((D.alertConfig || {}).waterValve ? '    <button class="tk-btn tk-btn--hold tk-btn--light" data-valve>' + App.icon('drop', 30) + '<span>Hold to shut off water</span></button>' : '') +
      '    <button class="tk-btn tk-btn--solid tk-btn--light" data-ack>' + App.icon('check', 30) + '<span>Acknowledge</span></button>' +
      '  </div>' +
      '</div>', 'takeover--alarm');
    App.alarm('leak');
    var v = el.querySelector('[data-valve]');
    if (v) App.hold(v, 1200, {
      done: function () {
        App.write('home', 'POST', '/home/water-off', {}).then(function () { v.classList.add('is-on'); v.querySelector('span').textContent = 'Water main closed'; });
      },
      cancel: function () { App.toast('Keep holding to shut off the water main', { ms: 2000, icon: 'drop' }); }
    });
    el.querySelector('[data-ack]').onclick = function () {
      closeTakeover();
      App.toast('Leak acknowledged', { icon: 'check' });
    };
  };

  // Smoke or carbon monoxide: list = [{ name, co }] of the detectors going off.
  App.smoke = function (list) {
    var now = App.now(), co = list.length && list.every(function (s) { return s.co; });
    var el = takeover(
      '<div class="tk-alarm">' +
      '  <div class="tk-alarm-icon">' + App.icon('flame', 88) + '</div>' +
      '  <div class="label label--light">Critical alert · ' + App.fmt.time(now) + '</div>' +
      '  <h1 class="tk-huge">' + (co ? 'Carbon monoxide' : 'Smoke alarm') + '</h1>' +
      '  <p class="tk-sub">' + App.esc(list.map(function (s) { return s.name; }).join(', ')) + '. ' +
      (co ? 'Open windows and get everyone outside.' : 'Check it now. If there’s a fire, get everyone out.') + '</p>' +
      '  <div class="tk-actions">' +
      '    <button class="tk-btn tk-btn--solid tk-btn--light" data-ack>' + App.icon('check', 30) + '<span>Acknowledge</span></button>' +
      '  </div>' +
      '</div>', 'takeover--alarm');
    App.alarm('smoke');
    el.querySelector('[data-ack]').onclick = function () {
      closeTakeover();
      App.toast((co ? 'CO' : 'Smoke') + ' alarm acknowledged', { icon: 'check' });
    };
  };

  // ------------------------------------------------------------ banners (amber, actionable)
  App.banner = function (opts) {
    var box = $('#banners');
    var b = document.createElement('div');
    b.className = 'banner banner--' + (opts.tone || 'amber');
    b.innerHTML = App.icon(opts.icon || 'bell', 30) + '<div class="banner-text"><b>' + opts.title + '</b><span>' + (opts.sub || '') + '</span></div>' +
      (opts.action ? '<button class="banner-btn banner-btn--solid" data-act>' + opts.action + '</button>' : '') +
      '<button class="banner-btn" data-x>' + (opts.dismiss || 'Dismiss') + '</button>';
    box.appendChild(b);
    b.classList.add('is-in');
    function kill() { b.remove(); }
    b.querySelector('[data-x]').onclick = kill;
    if (opts.action) b.querySelector('[data-act]').onclick = function () { kill(); opts.onAction(); };
    if (opts.ms) setTimeout(kill, opts.ms);
    return kill;
  };

  App.garageOpenAlert = function (mins) {
    App.banner({
      icon: 'garage', title: 'Garage open for ' + App.fmt.mins(mins || 15), sub: 'Close it from here if nobody needs it open',
      action: 'Close garage', dismiss: 'Leave open',
      onAction: function () { App.openGarageSheet(); }
    });
  };

  // ------------------------------------------------------------ display controls drawer
  function demoHTML() {
    return '<div class="demo-panel">' +
      '<div class="demo-head"><div><div class="label label--light">Home HQ</div><h2 class="serif">Display controls</h2></div><button class="demo-x" data-demo-close>' + App.icon('x', 28) + '</button></div>' +
      '<div class="demo-sec"><div class="label label--light">Modes</div><div class="demo-test">' +
      '<select id="demoMode" aria-label="Mode to show"><option value="ambient">Ambient</option><option value="off">Screen off</option></select>' +
      '<button data-modego>Show</button>' +
      '<button class="demo-icon" data-reload aria-label="Refresh data">' + App.icon('refresh', 26) + '</button></div></div>' +
      (tests().length ? '<div class="demo-sec"><div class="label label--light">Test alerts (nothing is sent to the house)</div><div class="demo-test">' +
      '<select id="demoAlert" aria-label="Alert to test">' + tests().map(function (t) { return '<option value="' + t[0] + '">' + t[1] + '</option>'; }).join('') + '</select>' +
      '<button data-test>Show</button></div></div>' : '') +
      '<div class="demo-sec"><div class="label label--light">Sound on this frame</div><div class="demo-sound">' +
      '<button data-mute class="' + (App.soundSettings.muted ? '' : 'is-on') + '">' + (App.soundSettings.muted ? 'Chimes off' : 'Chimes on') + '</button>' +
      '<input type="range" min="0" max="100" step="5" value="' + Math.round(App.soundSettings.volume * 100) + '" data-volume aria-label="Volume">' +
      '<span class="demo-vol num">' + Math.round(App.soundSettings.volume * 100) + '%</span></div>' +
      '<p class="demo-note">Doorbell, leave now and bins chimes; quiet at night and while everyone’s away. Smoke, CO and leak alarms always sound.</p></div>' +
      '<div class="demo-sec"><div class="label label--light">Night mode</div>' +
      '<div class="demo-night">' +
      '<label>Dim <input type="time" data-night="from" value="' + App.night.from + '" aria-label="Dim from"></label>' +
      '<label>Bright <input type="time" data-night="to" value="' + App.night.to + '" aria-label="Bright from"></label></div>' +
      '<div class="demo-night">' +
      '<label>Sleep <input type="time" data-night="offFrom" value="' + App.night.offFrom + '" aria-label="Screen off from"></label>' +
      '<label>Wake <input type="time" data-night="offTo" value="' + App.night.offTo + '" aria-label="Screen on from"></label>' +
      '<button data-nightoff class="' + (App.night.offOn ? 'is-on' : '') + '" aria-label="Screen off at night">' + (App.night.offOn ? 'On' : 'Off') + '</button></div></div>' +
      '<div class="demo-sec"><div class="label label--light">Timeouts (seconds)</div>' +
      '<div class="demo-night">' +
      '<label>Today <input type="number" inputmode="numeric" min="5" max="3600" step="5" data-idle="home" value="' + App.idle.home + '" aria-label="Seconds before returning to Today"></label>' +
      '<label>Rest <input type="number" inputmode="numeric" min="5" max="3600" step="5" data-idle="rest" value="' + App.idle.rest + '" aria-label="Seconds before the photo screen"></label></div>' +
      '<div class="demo-night">' +
      '<label>Peek <input type="number" inputmode="numeric" min="5" max="3600" step="5" data-idle="peek" value="' + App.idle.peek + '" aria-label="Seconds before the night clock"></label>' +
      '<span class="demo-spacer"></span></div></div>' +
      '<div class="demo-sec" id="demoAdb"></div>' +
      '<div class="demo-sec"><div class="label label--light">Frames</div><div class="demo-frames" id="demoFrames"><p class="demo-note">Checking…</p></div></div>' +
      '<div class="demo-sec"><div class="label label--light">Status</div><p class="demo-note">' + status() + '</p>' +
      '<p class="demo-note">Version ' + __APP_VERSION__ + '</p></div>' +
      '<p class="demo-note">Keyboard: <b>D</b> controls · <b>A</b> ambient · <b>1–6</b> views</p>' +
      '</div>';
  }
  // [id, label]: Home Assistant alerts only when it's set up; weather and bins work without it.
  function tests() {
    var t = D.hasHome ? [['doorbell', 'Doorbell'], ['leak', 'Water leak'], ['smoke', 'Smoke'], ['garage', 'Garage open']] : [];
    t.push(['weather', 'Weather warning']);
    if (D.bins && D.bins.next.length) t.push(['bins', 'Bins out tonight']);
    if (D.hasHome) t.push(['notify', 'Frame notification (sends one)']);
    t.push(['leavechime', 'Sound: leave now'], ['check', 'Sound check']);
    return t;
  }
  function status() {
    var bits = [D.events.length + ' calendar events', D.photos.length + ' photos', D.weather ? 'weather ok' : 'no weather'];
    (D.calendarErrors || []).forEach(function (e) { bits.push(App.esc(e)); });
    // Short form: "HA offline (unreachable)", not "Home Assistant offline (Home Assistant unreachable)".
    var haErr = (D.homeError || '').replace(/^Home Assistant:?\s*/, '');
    if (D.hasHome) bits.push(D.homeOnline ? 'HA connected' + (D.homeVia === 'push' ? ' (pushed)' : D.homeVia === 'read' ? ' (read by the Worker)' : '') : 'HA offline' + (haErr ? ' (' + App.esc(haErr) + ')' : ''));
    return bits.join(' · ');
  }
  // Every wall display that has checked in, newest software first; quiet for 15 min = a problem.
  function wifi(dbm) { return dbm == null ? '' : dbm >= -60 ? 'Wi-Fi good' : dbm >= -72 ? 'Wi-Fi fair' : 'Wi-Fi weak'; }
  function fillFrames() {
    App.api('GET', '/frames').then(function (list) {
      var box = $('#demoFrames');
      if (!box) return;
      var me = App.frameId ? App.frameId() : '';
      box.innerHTML = list.length ? list.map(function (f) {
        var mins = Math.round((Date.now() - f.seen) / 60000), quiet = mins >= 15;
        var bits = [f.version ? 'v' + f.version : '', f.kiosk ? 'kiosk ' + f.kiosk : 'kiosk too old to report',
          f.rssi != null ? wifi(f.rssi) + ' (' + f.rssi + ' dBm)' : '', mins < 1 ? 'just now' : mins < 120 ? mins + ' min ago' : Math.round(mins / 60) + ' h ago'];
        return '<p class="demo-frame' + (quiet ? ' is-quiet' : '') + '"><b>' + App.esc(f.name) + (f.id === me ? ' (this one)' : '') + '</b> ' +
          bits.filter(Boolean).map(App.esc).join(' · ') + '</p>';
      }).join('') : '<p class="demo-note">No frames have checked in yet.</p>';
    }).catch(function () {});
  }
  // Wireless ADB (kiosk 1.4+): switch it on for 5 minutes from here, so a computer on the allowed subnet
  // can deploy an update over Wi-Fi. The kiosk keeps the timer and the firewall; this shows the state,
  // the address to connect to, and counts down while the panel is open.
  var adbTimer;
  function fillAdb() {
    var el = $('#demoAdb'), k = App.kiosk();
    if (!el) return;
    if (!k || k.adbLeft === undefined) { el.innerHTML = ''; return; }
    var left = k.adbLeft, on = left > 0, clock = on ? Math.floor(left / 60) + ':' + ('0' + (left % 60)).slice(-2) : 'Off';
    // Rewriting the section every second replaced the button under a finger and lost the tap: only the
    // countdown text changes while the state is the same.
    var key = [on, k.adbFrom, k.adbIp, k.adbError].join('|');
    if (el.getAttribute('data-k') === key) { var c = el.querySelector('.demo-vol'); if (c) c.textContent = clock; return; }
    el.setAttribute('data-k', key);
    var from = k.adbFrom, err = k.adbError ? '<p class="demo-note" style="color:#FF8A80">' + App.esc(k.adbError) + '</p>' : '';
    if (!from) {
      // The kiosk won't start it without a subnet saved, so there's nothing to press.
      el.innerHTML = '<div class="label label--light">Wireless ADB</div>' +
        '<p class="demo-note">Needs a subnet saved first, over USB: <b>adb shell am start -n com.wesbos.homehq/.MainActivity --es adbfrom &lt;your computer’s subnet&gt;</b></p>' + err;
      return;
    }
    el.innerHTML = '<div class="label label--light">Wireless ADB</div>' +
      '<div class="demo-sound"><button data-adb class="' + (on ? 'is-on' : '') + '">' + (on ? 'Turn off' : 'Turn on for 5 min') + '</button>' +
      '<span class="demo-vol num">' + clock + '</span></div>' +
      '<p class="demo-note">' + (on
        ? 'adb connect ' + App.esc(k.adbIp || '<this frame’s address>') + ':5555 · ' + (from === 'any' ? '<b>open to this whole Wi-Fi</b>' : 'only from ' + App.esc(from))
        : 'For deploying an update over Wi-Fi. Switches itself off after 5 minutes. ' + (from === 'any' ? 'No address lock: anyone on this Wi-Fi could connect while it’s on.' : 'Only ' + App.esc(from) + ' can connect.')) + '</p>' + err;
  }
  App.openDemo = function () {
    var el = $('#demo');
    el.innerHTML = demoHTML();
    el.classList.add('is-open');
    fillFrames();
    fillAdb();
    clearInterval(adbTimer);
    adbTimer = setInterval(function () { if ($('#demo').classList.contains('is-open')) fillAdb(); else clearInterval(adbTimer); }, 1000);
  };
  function closeDemo() { $('#demo').classList.remove('is-open'); clearInterval(adbTimer); }

  function demoClick(e) {
    var b = e.target.closest('button');
    if (!b) { if (e.target === $('#demo')) closeDemo(); return; }
    if (b.hasAttribute('data-demo-close')) return closeDemo();
    if (b.hasAttribute('data-adb')) {
      var kk = App.kiosk(), K = window.HomeHQKiosk;
      if (kk && K && K.setAdb) {
        K.setAdb(!(kk.adbLeft > 0));
        // The kiosk sets it up in the background (a second or two): look again until it shows.
        [700, 1500, 3000].forEach(function (ms) { setTimeout(fillAdb, ms); });
      }
      return;
    }
    if (b.hasAttribute('data-modego')) {
      closeDemo();
      // Ambient = rest now, skipping the timeout: the clock still picks the screen (photos, dim
      // clock or screen off) and keeps switching at Dim, Bright, Sleep and Wake. Screen off is a
      // manual pick: as dark as the panel goes, and it stays until someone taps.
      if ($('#demoMode').value === 'off') App.setMode('off', true); else App.rest();
      return;
    }
    if (b.hasAttribute('data-mute')) {
      App.setSound({ muted: !App.soundSettings.muted });
      if (!App.soundSettings.muted) App.sound('check', true);
      App.openDemo();
      return;
    }
    if (b.hasAttribute('data-nightoff')) {
      App.setNight({ offOn: !App.night.offOn });
      App.openDemo();
      return;
    }
    if (b.hasAttribute('data-test')) {
      var which = $('#demoAlert').value;
      closeDemo();
      // Previews only: the leak valve and door lock still need a deliberate hold to act, and the
      // weather and bins tests stay on this screen for two minutes without saving anything.
      ({ doorbell: function () { App.doorbell(true); }, leak: App.leak,
        leavechime: function () { if (!App.sound('leave', true)) App.toast('Sounds are muted on this frame', { icon: 'bell' }); },
        check: function () { if (!App.sound('check', true)) App.toast('Sounds are muted on this frame', { icon: 'bell' }); }, smoke: function () { App.smoke([{ name: 'Test: Landing', co: false }]); },
        garage: function () { App.garageOpenAlert(15); }, weather: App.testWarning, bins: App.testBins,
        notify: function () {
          App.api('POST', '/frames/test-notify').then(function (r) { App.toast('Test sent to ' + r.sent + (r.sent === 1 ? ' device' : ' devices'), { icon: 'check' }); },
            function (err) { App.toast(App.esc(err.message), { tone: 'error', icon: 'bolt' }); });
        } })[which]();
      return;
    }
    if (b.hasAttribute('data-reload')) {
      closeDemo();
      Promise.all(['state', 'calendar', 'weather', 'home'].map(App.refresh)).then(function () { App.toast('Data refreshed', { icon: 'check', ms: 2000 }); });
    }
  }

  App.on('ready', function () {
    $('#demo').addEventListener('click', demoClick);
    // Time pickers: a tap anywhere on the box opens the picker, not just on its little icon.
    $('#demo').addEventListener('click', function (e) {
      var l = e.target.closest('.demo-night label'), i = l && l.querySelector('input');
      if (!i) return;
      e.preventDefault();
      if (i.showPicker) { try { i.showPicker(); return; } catch (err) {} } // throws if one is already open
      i.focus();
    });
    // Volume: follows the slider; a check sound when let go, so it can be set by ear.
    $('#demo').addEventListener('input', function (e) {
      if (!e.target.hasAttribute('data-volume')) return;
      App.setSound({ volume: +e.target.value / 100 });
      var lbl = $('#demo .demo-vol');
      if (lbl) lbl.textContent = e.target.value + '%';
    });
    $('#demo').addEventListener('change', function (e) {
      if (e.target.hasAttribute('data-volume')) App.sound('check', true);
      if (e.target.dataset.idle) {
        var q = {}; q[e.target.dataset.idle] = e.target.value;
        App.setIdle(q);
        e.target.value = App.idle[e.target.dataset.idle]; // out-of-range entries snap back
      }
      if (e.target.dataset.night && e.target.value) {
        var p = {}; p[e.target.dataset.night] = e.target.value;
        if (!App.setNight(p)) App.toast('Pick a time', { tone: 'error', icon: 'bolt' });
        else App.openDemo();
      }
    });
    App.hold($('#railClock'), 700, { done: App.openDemo });
    document.addEventListener('keydown', function (e) {
      if (e.target.tagName === 'INPUT') return;
      var k = e.key.toLowerCase();
      if (k === 'd') { $('#demo').classList.contains('is-open') ? closeDemo() : App.openDemo(); }
      if (k === 'a') App.rest();
      if (k === 'escape') { closeDemo(); App.closeSheet(); }
      var views = ['home', 'calendar', 'shopping', 'photos', 'control', 'media'];
      if (k >= '1' && k <= '6') App.go(views[+k - 1]);
    });
  });
})();
