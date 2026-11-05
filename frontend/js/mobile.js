// Phone page: the shopping list, live from the same API as the wall dashboard.
// Changes show on screen at once and save in the background; a poll that started before the last
// save is dropped, so a tick never flickers back.
var $ = function (s) { return document.querySelector(s); };
var S = { tab: 'shopping', store: 'all', data: null, wrote: 0, lastText: '' };
var POLL_MS = 20000;

// ------------------------------------------------------------ API
function api(method, path, body) {
  return fetch('/api' + path, {
    method: method, credentials: 'same-origin',
    headers: body ? { 'content-type': 'application/json' } : {},
    body: body ? JSON.stringify(body) : undefined
  }).then(function (r) {
    if (r.status === 401) { $('#locked').hidden = false; $('.tabs').hidden = true; throw new Error('locked'); }
    return r.json().catch(function () { return {}; }).then(function (j) {
      if (!r.ok) throw new Error(j.error || 'HTTP ' + r.status);
      return j;
    });
  });
}
function save(method, path, body) {
  S.wrote = Date.now();
  return api(method, path, body).then(function (r) { S.wrote = Date.now(); return r; }, function (err) {
    S.wrote = Date.now();
    toast('Couldn’t save · ' + err.message, null, true);
    load();
    throw err;
  });
}
// A wall display that hasn't checked in for 15 minutes is probably off, frozen or offline.
var framesAt = 0;
function checkFrames() {
  if (Date.now() - framesAt < 120e3) return;
  framesAt = Date.now();
  return api('GET', '/frames').then(function (list) {
    var quiet = list.filter(function (f) { return Date.now() - f.seen >= 15 * 60e3; });
    var box = $('#frameWarn');
    box.hidden = !quiet.length;
    box.innerHTML = quiet.map(function (f) {
      var mins = Math.round((Date.now() - f.seen) / 60000);
      return '<b>' + esc(f.name) + '</b> hasn’t checked in for ' + (mins < 120 ? mins + ' min' : Math.round(mins / 60) + ' hours') + '.';
    }).join('<br>') + (quiet.length ? ' Check it’s on and on Wi-Fi.' : '');
  }).catch(function () {});
}

function load() {
  var started = Date.now();
  checkFrames();
  return api('GET', '/state').then(function (d) {
    if (S.wrote >= started) return; // a save raced this poll
    var text = JSON.stringify(d);
    $('#sync').textContent = 'Updated ' + new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    if (text === S.lastText) return;
    S.lastText = text;
    S.data = d;
    if (document.activeElement !== $('#addInput')) render(); else renderList();
  }).catch(function (e) { if (e.message !== 'locked') $('#sync').textContent = 'Offline · retrying'; });
}
function uid() { return Date.now().toString(36) + Math.random().toString(36).slice(2, 8); }
function esc(s) { return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; }); }

// ------------------------------------------------------------ toast
var toastT;
function toast(text, undo, error) {
  var t = $('#toast');
  clearTimeout(toastT);
  t.className = 'toast' + (error ? ' is-error' : '');
  t.innerHTML = '<span>' + esc(text) + '</span>' + (undo ? '<button>Undo</button>' : '');
  t.hidden = false;
  if (undo) t.querySelector('button').onclick = function () { t.hidden = true; undo(); };
  toastT = setTimeout(function () { t.hidden = true; }, 4000);
}

// ------------------------------------------------------------ shopping
function storeName(id) { var s = S.data.stores.filter(function (x) { return x.id === id; })[0]; return s ? s.name : ''; }
function inStore(g) { return S.store === 'all' || g.store === S.store; }
function categorize(name) {
  var n = name.toLowerCase(), cat = S.data.catalog;
  var hit = cat.filter(function (c) { return c[0].toLowerCase() === n; })[0] ||
    cat.filter(function (c) { var a = c[0].toLowerCase(); return a.indexOf(n) > -1 || n.indexOf(a) > -1; })[0];
  return hit ? hit[1] : 'Other';
}
function putItem(g) { return save('POST', '/groceries', g); }

function renderStores() {
  var open = S.data.groceries.filter(function (g) { return !g.done; });
  $('#stores').innerHTML = [['all', 'All']].concat(S.data.stores.map(function (s) { return [s.id, s.name]; })).map(function (s) {
    var n = s[0] === 'all' ? open.length : open.filter(function (g) { return g.store === s[0]; }).length;
    return '<button data-store="' + s[0] + '" class="' + (S.store === s[0] ? 'is-on' : '') + '">' + esc(s[1]) + '<span class="n">' + n + '</span></button>';
  }).join('');
}

function itemHTML(g) {
  var tag = S.store === 'all' && S.data.stores.length > 1 ? '<span class="store-tag">' + esc(storeName(g.store)) + '</span>' : '';
  return '<div class="item' + (g.done ? ' is-done' : '') + '" data-id="' + g.id + '">' +
    '<button class="check" data-check aria-label="' + (g.done ? 'Add back' : 'Got it') + '">' + (g.done ? '✓' : '') + '</button>' +
    '<span class="label"><b>' + esc(g.name) + '</b>' + (g.note ? '<small>' + esc(g.note) + '</small>' : '') + '</span>' + tag +
    (g.done ? '' : '<span class="qty"><button data-qty="-1" aria-label="Fewer">−</button><span>' + g.qty + '</span><button data-qty="1" aria-label="More">+</button></span>') +
    '</div>';
}

function renderList() {
  if (!S.data) return;
  renderStores();
  var open = S.data.groceries.filter(function (g) { return !g.done && inStore(g); });
  var done = S.data.groceries.filter(function (g) { return g.done && inStore(g); });
  var order = (S.data.catOrder[S.store] || S.data.catOrder.all).filter(function (c) { return c !== 'Other'; }).concat(['Other']);
  var html = open.length ? order.map(function (cat) {
    var items = open.filter(function (g) { return g.cat === cat || (cat === 'Other' && order.indexOf(g.cat) < 0); });
    return items.length ? '<div class="group"><h2><span>' + esc(cat) + '</span><span>' + items.length + '</span></h2>' + items.map(itemHTML).join('') + '</div>' : '';
  }).join('') : '<div class="empty">All stocked up.</div>';
  if (done.length) {
    html += '<details class="got"' + (S.gotOpen ? ' open' : '') + '><summary>Got it (' + done.length + ')</summary>' +
      done.map(itemHTML).join('') + '<button class="link" data-clear>Clear ' + done.length + ' ticked items</button></details>';
  }
  $('#shopList').innerHTML = html;
  $('#catalog').innerHTML = S.data.catalog.map(function (c) { return '<option value="' + esc(c[0]) + '">'; }).join('');
}

function addItem(raw) {
  var name = raw.trim();
  if (!name) return;
  var list = S.data.groceries, lower = name.toLowerCase();
  var g = list.filter(function (x) { return !x.done && x.name.toLowerCase() === lower; })[0];
  if (g) { g.qty++; toast('Already on the list · now ×' + g.qty); }
  else if ((g = list.filter(function (x) { return x.done && x.name.toLowerCase() === lower; })[0])) { g.done = false; g.qty = 1; toast('Added back ' + g.name); }
  else {
    var cat = S.data.catalog.filter(function (c) { return c[0].toLowerCase() === lower; })[0];
    g = { id: uid(), name: cat ? cat[0] : name.charAt(0).toUpperCase() + name.slice(1), cat: categorize(name), qty: 1,
      store: S.store === 'all' ? S.data.stores[0].id : S.store, done: false, note: '' };
    list.push(g);
    toast('Added ' + g.name + (S.data.stores.length > 1 ? ' · ' + storeName(g.store) : ''));
  }
  renderList();
  putItem(g);
}

function onShopClick(e) {
  var t = e.target, row = t.closest('[data-id]');
  if (t.closest('[data-store]')) { S.store = t.closest('[data-store]').dataset.store; return renderList(); }
  if (t.closest('[data-clear]')) {
    var gone = S.data.groceries.filter(function (g) { return g.done && inStore(g); });
    var ids = gone.map(function (g) { return g.id; });
    S.data.groceries = S.data.groceries.filter(function (g) { return gone.indexOf(g) < 0; });
    renderList();
    save('POST', '/groceries/clear', { ids: ids });
    return toast('Cleared ' + ids.length + ' items', function () {
      S.data.groceries = S.data.groceries.concat(gone); renderList();
      save('POST', '/groceries/clear', { ids: ids, cleared: false });
    });
  }
  if (!row) return;
  var g = S.data.groceries.filter(function (x) { return x.id === row.dataset.id; })[0];
  if (!g) return;
  if (t.closest('[data-check]')) {
    g.done = !g.done;
    renderList(); putItem(g);
    if (g.done) toast(g.name + ' ticked off', function () { g.done = false; renderList(); putItem(g); });
    return;
  }
  var q = t.closest('[data-qty]');
  if (q) {
    g.qty = Math.max(1, g.qty + +q.dataset.qty);
    row.querySelector('.qty span').textContent = g.qty;
    clearTimeout(g._t);
    g._t = setTimeout(function () { delete g._t; putItem(g); }, 600);
  }
}

// ------------------------------------------------------------ tabs + boot
// ------------------------------------------------------------ sticky notes (shown on the frames)
var NS = { from: null, to: 'all', keep: 'cleared' };
try { NS.from = localStorage.getItem('hq-note-from'); } catch (e) {}
function segHTML(items, on, attr) {
  return items.map(function (x) { return '<button type="button" data-' + attr + '="' + x[0] + '" class="' + (x[0] === on ? 'is-on' : '') + '">' + esc(x[1]) + '</button>'; }).join('');
}
function renderNotes() {
  if (!S.data) return;
  var people = S.data.people || [];
  if (!NS.from || !people.some(function (p) { return p.id === NS.from; })) NS.from = people[0] && people[0].id;
  var names = people.map(function (p) { return [p.id, p.name]; });
  $('#noteFrom').innerHTML = segHTML(names, NS.from, 'from');
  $('#noteFor').innerHTML = segHTML([['all', 'Everyone']].concat(names), NS.to, 'to');
  $('#noteKeep').innerHTML = segHTML([['cleared', 'Until cleared'], ['tonight', 'Tonight'], ['tomorrow', 'Tomorrow evening']], NS.keep, 'keep');
  var notes = S.data.notes || [], who = function (id) { var p = people.filter(function (x) { return x.id === id; })[0]; return p ? p.name : id; };
  $('#noteList').innerHTML = notes.length ? '<div class="group"><h2><span>On the frames</span><span>' + notes.length + '</span></h2>' + notes.map(function (n) {
    return '<div class="note-card" data-note="' + n.id + '"><p class="serif">' + esc(n.text) + '</p><small>– ' + esc(who(n.from)) +
      (n.to !== 'all' ? ' · for ' + esc(who(n.to)) : '') + '</small><button type="button" data-clear>Done</button></div>';
  }).join('') + '</div>' : '<div class="empty">No notes on the frames.</div>';
}

function render() { renderList(); renderNotes(); }
function showTab(name) {
  S.tab = name;
  $('#title').textContent = name === 'notes' ? 'Notes' : 'Shopping';
  $('#tab-shopping').hidden = name !== 'shopping';
  $('#tab-notes').hidden = name !== 'notes';
  Array.prototype.forEach.call(document.querySelectorAll('.tabs [data-tab]'), function (b) { b.classList.toggle('is-on', b.dataset.tab === name); });
}

document.querySelector('.tabs').addEventListener('click', function (e) {
  var b = e.target.closest('[data-tab]');
  if (b) showTab(b.dataset.tab);
});
$('#tab-notes').addEventListener('click', function (e) {
  var b = e.target.closest('button');
  if (!b) return;
  ['from', 'to', 'keep'].forEach(function (k) { if (b.dataset[k] != null) { NS[k] = b.dataset[k]; renderNotes(); } });
  if (b.dataset.from) try { localStorage.setItem('hq-note-from', b.dataset.from); } catch (err) {}
  if (b.hasAttribute('data-clear')) {
    var id = b.closest('[data-note]').dataset.note;
    S.data.notes = S.data.notes.filter(function (n) { return n.id !== id; });
    renderNotes();
    save('DELETE', '/notes/' + id).then(function (r) { S.data.notes = r.notes; renderNotes(); });
  }
});
$('#noteForm').addEventListener('submit', function (e) {
  e.preventDefault();
  var text = $('#noteText').value.trim();
  if (!text) return $('#noteText').focus();
  var mid = new Date(); mid.setHours(24, 0, 0, 0);
  var expires = NS.keep === 'tonight' ? +mid : NS.keep === 'tomorrow' ? +mid + 20 * 36e5 : null;
  var note = { id: uid(), text: text, from: NS.from, to: NS.to, expires: expires, created: Date.now() };
  S.data.notes = [note].concat(S.data.notes || []);
  $('#noteText').value = '';
  renderNotes();
  save('POST', '/notes', note).then(function (r) { S.data.notes = r.notes; renderNotes(); toast('Pinned on the frames'); });
});
$('#addForm').addEventListener('submit', function (e) {
  e.preventDefault();
  var i = $('#addInput');
  addItem(i.value);
  i.value = '';
});
$('#tab-shopping').addEventListener('click', onShopClick);
$('#shopList').addEventListener('toggle', function (e) { if (e.target.classList.contains('got')) S.gotOpen = e.target.open; }, true);
document.addEventListener('visibilitychange', function () { if (!document.hidden) load(); });

showTab('shopping');
load();
// Registers the share target (Share → Home HQ in the gallery) once the app is installed.
if ('serviceWorker' in navigator) navigator.serviceWorker.register('/share-sw.js').catch(function () {});
setInterval(function () { if (!document.hidden) load(); }, POLL_MS);
