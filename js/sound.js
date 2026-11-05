// Sounds, made with the Web Audio API (no audio files). Two kinds:
//   alarms (smoke, CO, water leak): repeat until the takeover closes, always play, even muted;
//   chimes (doorbell, leave now, bins, sound check): one-offs, quiet when this frame is muted,
//   at night (the frame's night hours, 10 PM – 6 AM unless set, or night mode) and while everyone is away (screen off).
// Mute and volume are per frame (localStorage), set in the display controls. The kiosk lets
// pages play sound without a tap first; in a browser the first tap unlocks it.
(function () {
  'use strict';
  var D = window.DATA;
  var ctx = null, alarmTimer = null;

  function load(k, d) { try { var v = localStorage.getItem(k); return v == null ? d : JSON.parse(v); } catch (e) { return d; } }
  function save(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch (e) {} }
  var S = App.soundSettings = { muted: load('hq-muted', false), volume: load('hq-volume', 0.8) };
  App.setSound = function (patch) {
    Object.assign(S, patch);
    save('hq-muted', S.muted); save('hq-volume', S.volume);
  };

  function audio() {
    var AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return null;
    if (!ctx) ctx = new AC();
    if (ctx.state === 'suspended') ctx.resume();
    return ctx;
  }
  document.addEventListener('pointerdown', function () { if (ctx && ctx.state === 'suspended') ctx.resume(); }, true);

  // One note: frequency (Hz), start and length (s) from now, wave shape, loudness 0–1.
  function note(c, out, f, at, len, type, level) {
    var o = c.createOscillator(), g = c.createGain(), t = c.currentTime + at;
    o.type = type || 'sine';
    o.frequency.value = f;
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(level || 0.5, t + 0.015);
    g.gain.exponentialRampToValueAtTime(0.0001, t + len);
    o.connect(g); g.connect(out);
    o.start(t); o.stop(t + len + 0.05);
  }
  var SOUNDS = {
    doorbell: function (c, o) { note(c, o, 659.3, 0, 1.3, 'sine', 0.6); note(c, o, 523.3, 0.55, 1.8, 'sine', 0.6); },
    leave: function (c, o) { note(c, o, 523.3, 0, 0.35); note(c, o, 659.3, 0.13, 0.35); note(c, o, 784, 0.26, 0.6); },
    bins: function (c, o) { note(c, o, 392, 0, 0.7, 'sine', 0.35); note(c, o, 523.3, 0.28, 0.9, 'sine', 0.35); },
    check: function (c, o) { note(c, o, 880, 0, 0.5, 'sine', 0.5); },
    note: function (c, o) { note(c, o, 784, 0, 0.25, 'sine', 0.35); note(c, o, 1046.5, 0.12, 0.5, 'sine', 0.35); },
    smoke: function (c, o) { [0, 0.3, 0.6].forEach(function (t) { note(c, o, 3100, t, 0.18, 'square', 0.25); }); },
    leak: function (c, o) { note(c, o, 880, 0, 0.3, 'triangle', 0.5); note(c, o, 660, 0.35, 0.3, 'triangle', 0.5); }
  };
  var ALARMS = { smoke: 1.6, leak: 1.4 }; // repeat every n seconds

  function play(name) {
    var c = audio();
    if (!c || !SOUNDS[name]) return;
    var out = c.createGain();
    out.gain.value = Math.max(0, Math.min(1, S.volume));
    out.connect(c.destination);
    SOUNDS[name](c, out);
  }
  function quietNow() {
    return S.muted || App.isNight() || App.mode === 'night' || App.mode === 'off' ||
      (App.everyoneAway && App.everyoneAway());
  }

  // A chime: skipped when muted, at night or while everyone is away. force: a test, which plays
  // unless muted, so the display controls can check the volume at any hour.
  App.sound = function (name, force) {
    if (force ? S.muted : quietNow()) return false;
    play(name);
    return true;
  };
  // An alarm: always plays (the frame adds to the real alarms, never replaces them), repeating
  // until App.stopAlarm(), which closing the takeover calls.
  App.alarm = function (name) {
    App.stopAlarm();
    play(name);
    alarmTimer = setInterval(function () { play(name); }, ALARMS[name] * 1000);
  };
  App.stopAlarm = function () { clearInterval(alarmTimer); alarmTimer = null; };
})();
