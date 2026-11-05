// Media: every media player Home Assistant has (merged by name, unavailable ones hidden; see
// playersView in the Worker), playing ones first. Each card: artwork, what's on, play/pause,
// previous/next, volume, mute and power, offered only where the player supports them.
(function () {
  'use strict';
  var D = window.DATA, V = {};
  D.players = D.players || []; // sync.js may already have restored them from the saved copy
  // Home Assistant's supported_features bits
  var F = { PAUSE: 1, VOLUME: 4, MUTE: 8, PREV: 16, NEXT: 32, ON: 128, OFF: 256, PLAY: 16384 };
  var has = function (p, f) { return (p.features & f) === f; };
  var STATE = { playing: 'Playing', paused: 'Paused', buffering: 'Loading', idle: 'Idle', on: 'On', standby: 'Standby', off: 'Off' };

  function card(p) {
    var on = p.state !== 'off' && p.state !== 'standby', playing = p.state === 'playing', loaded = p.loaded !== false;
    var what = p.title ? App.esc(p.title) : on ? '<i>Nothing playing</i>' : '<i>Switched off</i>';
    var sub = [p.artist, p.app].filter(Boolean).map(App.esc).join(' · ');
    var btn = function (act, icon, label, ok, cls) {
      return '<button class="md-btn' + (cls || '') + '" data-do="' + act + '" aria-label="' + label + '"' + (ok ? '' : ' disabled') + '>' + App.icon(icon, 28) + '</button>';
    };
    return '<div class="md-card' + (playing ? ' is-playing' : '') + (on ? '' : ' is-off') + '" data-player="' + p.id + '">' +
      '<div class="md-top">' +
      (p.art && on ? '<img class="md-art" src="' + p.art + '" alt="">' : '<span class="md-art md-art--none">' + App.icon('music', 40) + '</span>') +
      '<div class="md-text"><span class="label">' + App.esc(p.name) + ' · ' + (STATE[p.state] || p.state) + '</span>' +
      '<b class="serif">' + what + '</b>' + (sub ? '<span>' + sub + '</span>' : on && !loaded ? '<span>Nothing to resume: start something on the speaker or in Music Assistant</span>' : '') + '</div>' +
      ((has(p, F.ON) || has(p, F.OFF)) ? '<button class="md-power' + (on ? ' is-on' : '') + '" data-do="' + (on ? 'off' : 'on') + '" aria-label="' + (on ? 'Turn off' : 'Turn on') + '">' + App.icon('power', 24) + '</button>' : '') +
      '</div>' +
      (on ? '<div class="md-controls">' +
        btn('previous', 'prev', 'Previous', loaded && has(p, F.PREV)) +
        btn('play_pause', playing ? 'pause' : 'play', playing ? 'Pause' : 'Play', loaded && (has(p, F.PAUSE) || has(p, F.PLAY)), ' md-btn--main') +
        btn('next', 'next', 'Next', loaded && has(p, F.NEXT)) +
        (has(p, F.VOLUME) && p.volume != null ? '<div class="md-vol">' +
          (has(p, F.MUTE) ? '<button class="md-mute" data-do="mute" aria-label="' + (p.muted ? 'Unmute' : 'Mute') + '">' + App.icon(p.muted ? 'muted' : 'speaker', 24) + '</button>' : App.icon('speaker', 24)) +
          '<input type="range" min="0" max="100" step="2" value="' + p.volume + '" data-vol aria-label="Volume">' +
          '<span class="num">' + (p.muted ? 'Muted' : p.volume + '%') + '</span></div>' : '') +
        '</div>' : '') +
      '</div>';
  }

  function render() {
    if (!V.el) return;
    if (V.dragging) return; // don't move a slider under someone's finger
    V.el.innerHTML = '<div class="md">' +
      '<div class="md-head"><h2 class="serif">Media</h2><span class="label">' + D.players.filter(function (p) { return p.state === 'playing'; }).length + ' playing</span></div>' +
      (D.players.length ? '<div class="md-grid">' + D.players.map(card).join('') + '</div>'
        : '<div class="md-empty serif"><i>No media players</i> reachable in Home Assistant.</div>') +
      '</div>';
  }

  // Optimistic: change the card now, send to Home Assistant, and let the next poll correct it.
  function act(id, what, value) {
    var p = D.players.filter(function (x) { return x.id === id; })[0];
    if (!p) return;
    if (what === 'play_pause') p.state = p.state === 'playing' ? 'paused' : 'playing';
    if (what === 'off') p.state = 'off';
    if (what === 'on') p.state = 'idle';
    if (what === 'mute') { p.muted = !p.muted; value = p.muted; }
    if (what === 'volume') { p.volume = value; p.muted = false; }
    render();
    // A refusal (nothing to resume) shows as a toast from App.write and the poll puts the card back.
    App.write('home', 'POST', '/home/media/' + id, { do: what, value: value }).then(function () { setTimeout(function () { App.refresh('home'); }, 1200); }, function () {});
  }

  App.register('media', {
    show: function (el) {
      if (!V.el) {
        V.el = el;
        el.addEventListener('click', function (e) {
          var b = e.target.closest('[data-do]');
          if (b && !b.disabled) act(b.closest('[data-player]').dataset.player, b.dataset.do);
        });
        el.addEventListener('pointerdown', function (e) { if (e.target.matches('[data-vol]')) V.dragging = true; });
        el.addEventListener('input', function (e) {
          if (!e.target.matches('[data-vol]')) return;
          e.target.nextElementSibling.textContent = e.target.value + '%';
        });
        el.addEventListener('change', function (e) {
          if (!e.target.matches('[data-vol]')) return;
          V.dragging = false;
          act(e.target.closest('[data-player]').dataset.player, 'volume', +e.target.value);
        });
      }
      render();
    }
  });
  App.on('data', function () { if (App.current === 'media') render(); });
})();
