// The calendar page: connect CalDAV servers (Nextcloud Login Flow v2, or an app password), add ICS
// links, and choose per calendar whose it is, whether it shows and whether the frame adds events to
// it. App passwords and ICS links go to the backend and never come back.
var $ = function (id) { return document.getElementById(id); };
var state = { people: [], accounts: [] };
var icsPeople = [];
var timer = null;

function esc(s) {
  return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]; });
}

async function api(method, path, body) {
  var opts = { method: method, credentials: 'same-origin', cache: 'no-store', headers: {} };
  if (body !== undefined) { opts.headers['content-type'] = 'application/json'; opts.body = JSON.stringify(body); }
  var r = await fetch('/api/calendars' + (path || ''), opts);
  if (r.status === 401) throw new Error('Unlock this display with its Home HQ key, then try again.');
  var j = await r.json().catch(function () { return {}; });
  if (!r.ok) throw new Error(j.error || 'Something went wrong (HTTP ' + r.status + ').');
  return j;
}

function showError(e) { $('error').textContent = e ? e.message || String(e) : ''; }

// Busy while a request runs, so a second tap doesn't send it twice.
async function busy(button, fn) {
  if (button.disabled) return;
  button.disabled = true; showError(null);
  try { await fn(); } catch (e) { showError(e); } finally { button.disabled = false; }
}

function chips(selected, onToggle) {
  var everyone = !selected.length || selected.indexOf('all') > -1;
  var html = state.people.map(function (p) {
    var on = !everyone && selected.indexOf(p.id) > -1;
    return '<button class="chip' + (on ? ' on' : '') + '" data-person="' + esc(p.id) + '" style="--c:' + esc(p.color || '#171917') + '">' + esc(p.name) + '</button>';
  }).join('') + '<button class="chip' + (everyone ? ' on' : '') + '" data-person="all">Everyone</button>';
  var box = document.createElement('div');
  box.className = 'chips';
  box.innerHTML = html;
  box.addEventListener('click', function (ev) {
    var b = ev.target.closest('[data-person]');
    if (!b) return;
    var id = b.dataset.person, next;
    if (id === 'all') next = ['all'];
    else {
      next = everyone ? [] : selected.filter(function (x) { return x !== 'all'; });
      next = next.indexOf(id) > -1 ? next.filter(function (x) { return x !== id; }) : next.concat([id]);
      if (!next.length) next = ['all'];
    }
    onToggle(next);
  });
  return box;
}

function renderIcsPeople() {
  var box = $('ics-people');
  box.replaceWith(Object.assign(chips(icsPeople, function (next) { icsPeople = next; renderIcsPeople(); }), { id: 'ics-people' }));
}

function renderAccounts() {
  var list = $('accounts');
  list.innerHTML = '';
  if (!state.accounts.length) {
    list.innerHTML = '<p class="muted">No calendars added here yet: connect a server or add a link below.</p>';
  }
  state.accounts.forEach(function (a) {
    var el = document.createElement('div');
    el.className = 'account';
    var who = a.kind === 'ics' ? 'ICS link' : esc(a.username) + ' · ' + (a.via === 'login flow' ? 'signed in on a phone' : 'app password');
    el.innerHTML = '<div class="account-head"><div><h2>' + esc(a.name) + '</h2><div class="note">' + who + '</div></div>' +
      '<div class="row">' + (a.kind === 'caldav' ? '<button class="ghost small" data-refresh>Look for new calendars</button>' : '') +
      '<button class="ghost small" data-remove>Remove</button></div></div>';
    a.calendars.forEach(function (c) {
      var row = document.createElement('div');
      row.className = 'cal';
      var canAdd = a.kind === 'caldav' && c.serverWritable;
      row.innerHTML = '<div class="cal-name">' + esc(c.name) + '</div>' +
        '<div class="toggles"><label><input type="checkbox" data-show' + (c.show ? ' checked' : '') + '> Show on the frame</label>' +
        '<label><input type="checkbox" data-add' + (c.addEvents ? ' checked' : '') + (canAdd ? '' : ' disabled') + '> Add events from the frame here</label>' +
        (a.kind === 'caldav' && !c.serverWritable ? '<span class="note">read-only for this account</span>' : '') + '</div>';
      row.insertBefore(chips(c.people, function (next) { patch(a, c, { people: next }); }), row.querySelector('.toggles'));
      row.querySelector('[data-show]').addEventListener('change', function (ev) { patch(a, c, { show: ev.target.checked }); });
      row.querySelector('[data-add]').addEventListener('change', function (ev) { patch(a, c, { addEvents: ev.target.checked }); });
      el.appendChild(row);
    });
    var refresh = el.querySelector('[data-refresh]');
    if (refresh) refresh.addEventListener('click', function () {
      busy(refresh, async function () { replace(await api('POST', '/' + a.id + '/refresh')); });
    });
    var remove = el.querySelector('[data-remove]');
    remove.addEventListener('click', function () {
      var extra = a.via === 'login flow' ? ' Then revoke the "Home HQ" app password on the server.' : '';
      if (!window.confirm('Remove ' + a.name + ' from Home HQ?' + extra)) return;
      busy(remove, async function () { await api('DELETE', '/' + a.id); await load(); });
    });
    list.appendChild(el);
  });
}

function replace(account) {
  state.accounts = state.accounts.map(function (x) { return x.id === account.id ? account : x; });
  renderAccounts();
}

async function patch(a, c, change) {
  try { showError(null); replace(await api('PATCH', '/' + a.id + '/' + c.id, change)); }
  catch (e) { showError(e); renderAccounts(); }
}

function renderLogin(login) {
  clearTimeout(timer);
  $('pending').hidden = !login;
  $('connect-form').hidden = !!login;
  if (!login) { $('qr').removeAttribute('src'); return; }
  $('login-link').href = login.url;
  $('pending-text').textContent = 'Scan the code with your phone, sign in to ' + login.server + ' and approve Home HQ.';
  $('qr').src = '/api/calendars/login/qr?t=' + Date.now();
  timer = setTimeout(poll, 2000);
}

async function poll() {
  try {
    var r = await api('POST', '/login/poll');
    if (r.status === 'waiting') { timer = setTimeout(poll, 2000); return; }
    renderLogin(null);
    if (r.status === 'connected') {
      $('connect-status').innerHTML = '<span class="ok">Connected. Pick whose each calendar is, above.</span>';
      await load();
      window.scrollTo({ top: 0, behavior: 'smooth' });
    } else if (r.status === 'expired') {
      $('connect-status').innerHTML = '<span class="error">The login expired or was declined. Connect again to retry.</span>';
    }
  } catch (e) {
    renderLogin(null);
    showError(e);
  }
}

function render(v) {
  state = v;
  $('persist').hidden = !v.persistNote;
  $('persist').textContent = v.persistNote ? v.persistNote + '.' : '';
  $('env').hidden = !v.fromEnv;
  $('env').textContent = v.fromEnv ? v.fromEnv + ' more calendar' + (v.fromEnv === 1 ? ' comes' : 's come') + ' from the server configuration (HQ_CALENDARS) and can’t be changed here.' : '';
  renderIcsPeople();
  renderAccounts();
  renderLogin(v.login);
}

async function load() { render(await api('GET')); }

$('connect').addEventListener('click', function () {
  busy($('connect'), async function () {
    $('connect-status').textContent = '';
    renderLogin(await api('POST', '/login', { server: $('server').value }));
  });
});
$('cancel').addEventListener('click', function () {
  busy($('cancel'), async function () { await api('DELETE', '/login'); renderLogin(null); });
});
$('pw-add').addEventListener('click', function () {
  busy($('pw-add'), async function () {
    await api('POST', '/password', { server: $('pw-server').value, username: $('pw-user').value, password: $('pw-pass').value });
    $('pw-pass').value = '';
    $('connect-status').innerHTML = '<span class="ok">Added. Pick whose each calendar is, above.</span>';
    await load();
    window.scrollTo({ top: 0, behavior: 'smooth' });
  });
});
$('ics-add').addEventListener('click', function () {
  busy($('ics-add'), async function () {
    await api('POST', '/ics', { url: $('ics-url').value, name: $('ics-name').value, people: icsPeople });
    $('ics-url').value = ''; $('ics-name').value = ''; icsPeople = [];
    await load();
  });
});

load().catch(showError);
