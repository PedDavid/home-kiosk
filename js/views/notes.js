// Sticky notes: short notes for the household, pinned on every frame until someone taps Done or
// they expire. On Today they float bottom-left over the person lanes (three at most, newest on
// top); on other tabs a small pill; in ambient mode the newest shows small on the photo; hidden at
// night and with the screen off. Written here (the rail's note button) or on the phone page.
(function () {
  'use strict';
  var D = window.DATA, $ = App.$;
  D.notes = D.notes || []; // sync.js may already have restored them from the saved copy
  var seen = null; // ids already shown, so only new arrivals chime
  var NOTE_MAX = 140;

  function load(k, d) { try { var v = localStorage.getItem(k); return v == null ? d : JSON.parse(v); } catch (e) { return d; } }
  function save(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch (e) {} }

  function when(ms) {
    var d = new Date(ms), now = App.now();
    return App.sameDay(d, now) ? App.fmt.time(d) : App.fmt.dayShort(d) + ' ' + App.fmt.time(d);
  }
  function noteHTML(n, i) {
    var from = App.person(n.from), to = n.to !== 'all' ? App.person(n.to) : null;
    return '<button class="nt" data-note="' + n.id + '" style="' + App.pstyle(from) + ';--tilt:' + [-1.6, 1.2, -0.6][i % 3] + 'deg;--i:' + i + '">' +
      (to ? '<span class="nt-for">For ' + App.esc(to.name) + '</span>' : '') +
      '<span class="nt-text serif">' + App.esc(n.text) + '</span>' +
      '<span class="nt-by">– ' + App.esc(from.name) + ' · ' + when(n.created) + '</span></button>';
  }

  function render() {
    var el = $('#notes');
    if (!el) return;
    var list = D.notes, home = App.current === 'home';
    // Ambient: the newest, small, on the photo (and nothing once the last one is cleared).
    var amb = $('#ambient .amb-note');
    if (!amb && $('#ambient')) { amb = document.createElement('div'); amb.className = 'amb-note'; $('#ambient').appendChild(amb); }
    if (amb) amb.innerHTML = list[0] ? '<span class="serif">' + App.esc(list[0].text) + '</span><small>– ' + App.esc(App.person(list[0].from).name) + '</small>' : '';
    el.className = 'notes' + (list.length ? '' : ' is-empty') + (home ? ' is-home' : ' is-pill');
    if (!list.length) { el.innerHTML = ''; return; }
    if (home) {
      var shown = list.slice(0, 3);
      el.innerHTML = '<div class="nt-stack">' + shown.slice().reverse().map(function (n) { return noteHTML(n, shown.indexOf(n)); }).join('') + '</div>' +
        (list.length > 3 ? '<button class="nt-more" data-notes-all>+' + (list.length - 3) + ' more</button>' : '');
    } else {
      el.innerHTML = '<button class="nt-pill" data-notes-all>' + App.icon('note', 22) + list.length + (list.length === 1 ? ' note' : ' notes') + '</button>';
    }
  }

  // ------------------------------------------------------------ data
  App.on('data', function () {
    var ids = D.notes.map(function (n) { return n.id; });
    if (seen) { if (ids.some(function (id) { return seen.indexOf(id) < 0; })) App.sound('note'); }
    seen = ids;
    render();
  });
  App.on('view', render);
  App.on('mode', render);

  function apply(r) { if (r && r.notes) { D.notes = r.notes; seen = D.notes.map(function (n) { return n.id; }); render(); } }

  // ------------------------------------------------------------ write / edit
  App.noteSheet = function (n) {
    var mine = load('hq-my-notes', []);
    var st = n ? { from: n.from, to: n.to, keep: 'same' } : { from: load('hq-note-from', D.people[0] && D.people[0].id), to: 'all', keep: 'cleared' };
    var now = App.now(), tonight = App.addDays(App.dayStart(now), 1), tomorrowEve = new Date(+tonight + 20 * 36e5);
    var keeps = [['cleared', 'Until cleared'], ['tonight', 'Tonight'], ['tomorrow', 'Tomorrow evening'], ['date', 'Until a date']];
    var chip = function (attr, val, label, on, style) { return '<button class="chip' + (on ? ' is-on' : '') + '" data-' + attr + '="' + val + '"' + (style ? ' style="' + style + '"' : '') + '>' + label + '</button>'; };
    App.sheet('<div class="sheet-pad add ntw">' +
      '<div class="label">' + (n ? 'Edit note' : 'New note') + '</div>' +
      '<textarea class="big-input serif ntw-text" maxlength="' + NOTE_MAX + '" rows="2" placeholder="Leave a note…" data-f="text">' + (n ? App.esc(n.text) : '') + '</textarea>' +
      '<div class="ntw-count num" data-count></div>' +
      '<div class="add-row"><div><div class="label">From</div><div class="chips">' + D.people.map(function (p) { return chip('from', p.id, App.esc(p.name), st.from === p.id); }).join('') + '</div></div>' +
      '<div><div class="label">For</div><div class="chips">' + chip('to', 'all', 'Everyone', st.to === 'all') + D.people.map(function (p) { return chip('to', p.id, App.esc(p.name), st.to === p.id); }).join('') + '</div></div></div>' +
      (n ? '' : '<div class="label">Keep it</div><div class="chips">' + keeps.map(function (k) { return chip('keep', k[0], k[1], st.keep === k[0]); }).join('') +
        '<input type="date" class="chip chip-date" data-f="until" hidden></div>') +
      '<div class="sheet-actions">' + (n ? '<button class="btn btn--ghost" data-done>Done · clear it</button>' : '<button class="btn btn--ghost" data-close>Cancel</button>') +
      '<button class="btn btn--solid" data-save>' + (n ? 'Save' : 'Pin note') + '</button></div></div>', {
      cls: 'sheet--wide',
      onMount: function (s, close) {
        var ta = s.querySelector('[data-f=text]'), count = s.querySelector('[data-count]'), until = s.querySelector('[data-f=until]');
        var upd = function () { count.textContent = ta.value.length + ' / ' + NOTE_MAX; };
        ta.addEventListener('input', upd); upd();
        if (!n) setTimeout(function () { ta.focus(); }, 300);
        s.addEventListener('click', function (ev) {
          var t = ev.target.closest('button');
          if (!t) return;
          ['from', 'to', 'keep'].forEach(function (k) {
            if (t.dataset[k] == null) return;
            st[k] = t.dataset[k];
            App.$$('[data-' + k + ']', s).forEach(function (b) { b.classList.toggle('is-on', b === t); });
            if (k === 'keep' && until) { until.hidden = st.keep !== 'date'; if (st.keep === 'date' && until.showPicker) try { until.showPicker(); } catch (e) {} }
          });
          if (t.hasAttribute('data-done')) { close(); return App.clearNote(n); }
          if (!t.hasAttribute('data-save')) return;
          var text = ta.value.trim();
          if (!text) { ta.focus(); return App.toast('Write the note first', { icon: 'note', ms: 2500 }); }
          var expires = n ? n.expires : st.keep === 'tonight' ? +tonight : st.keep === 'tomorrow' ? +tomorrowEve :
            st.keep === 'date' && until.value ? +new Date(until.value + 'T23:59') : null;
          var note = { id: n ? n.id : App.uid(), text: text, from: st.from, to: st.to, expires: expires, created: n ? n.created : Date.now() };
          if (!n) { mine.push(note.id); save('hq-my-notes', mine.slice(-50)); save('hq-note-from', st.from); }
          D.notes = [note].concat(D.notes.filter(function (x) { return x.id !== note.id; }));
          seen = (seen || []).concat(note.id);
          close(); App.go('home'); render();
          App.write('state', 'POST', '/notes', note).then(apply, function () {});
          App.toast(n ? 'Note saved' : 'Note pinned', { icon: 'note', ms: 2500 });
        });
      }
    });
  };

  // A note full size: Done (clears it on every frame), Edit only where it was written.
  function openNote(n) {
    var from = App.person(n.from), mine = load('hq-my-notes', []).indexOf(n.id) > -1;
    App.sheet('<div class="sheet-pad ntv" style="' + App.pstyle(from) + '">' +
      '<div class="label">' + (n.to !== 'all' ? 'For ' + App.esc(App.person(n.to).name) + ' · ' : '') + 'Note from ' + App.esc(from.name) + ' · ' + when(n.created) + '</div>' +
      '<p class="ntv-text serif">' + App.esc(n.text) + '</p>' +
      (n.expires ? '<div class="label">Clears itself ' + when(n.expires) + '</div>' : '') +
      '<div class="sheet-actions">' + (mine ? '<button class="btn btn--ghost" data-edit>Edit</button>' : '<span></span>') +
      '<button class="btn btn--solid" data-done>Done · clear it</button></div></div>', {
      onMount: function (s, close) {
        s.addEventListener('click', function (ev) {
          if (ev.target.closest('[data-done]')) { close(); App.clearNote(n); }
          if (ev.target.closest('[data-edit]')) { close(); App.noteSheet(n); }
        });
      }
    });
  }
  function allNotes() {
    App.sheet('<div class="sheet-pad ntl"><div class="label">Notes</div>' + D.notes.map(function (n, i) { return noteHTML(n, i); }).join('') + '</div>', {
      cls: 'sheet--wide',
      onMount: function (s, close) { s.addEventListener('click', function (ev) { var b = ev.target.closest('[data-note]'); if (b) { close(); openNote(App.findNote(b.dataset.note)); } }); }
    });
  }
  App.findNote = function (id) { return D.notes.filter(function (n) { return n.id === id; })[0]; };
  App.clearNote = function (n) {
    D.notes = D.notes.filter(function (x) { return x.id !== n.id; });
    render();
    App.write('state', 'DELETE', '/notes/' + n.id).then(apply, function () {});
    App.toast('Note cleared', { icon: 'note', undo: function () {
      D.notes = [n].concat(D.notes); render();
      App.write('state', 'POST', '/notes/' + n.id + '/restore').then(apply, function () {});
    } });
  };

  App.on('ready', function () {
    $('#notes').addEventListener('click', function (ev) {
      var b = ev.target.closest('[data-note]');
      if (b) return openNote(App.findNote(b.dataset.note));
      if (ev.target.closest('[data-notes-all]')) return D.notes.length === 1 ? openNote(D.notes[0]) : allNotes();
    });
    var rb = $('.rail-item[data-note-new]');
    if (rb) rb.addEventListener('click', function () { App.noteSheet(); });
    render();
  });
})();
