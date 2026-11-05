// Home HQ backend: serves the built dashboard and a small JSON API.
//   /api/state     lists (shopping, photos) + family config                D1
//   /api/calendar  Google Calendar ICS feeds + events added on the frame    ICS → D1 cache
//   /api/weather   Open-Meteo forecast                                      → D1 cache
//   /api/home      Home Assistant states, controls and camera snapshots     HA REST API
//   /api/photos    upload / serve / favourite / delete                      R2 + D1
// Every /api route needs the dashboard key: open any page once with ?key=<DASHBOARD_KEY> and it is
// kept in a cookie (the kiosk WebView keeps cookies across reboots).
import RAW from '../settings.json';
import { parseIcs } from './ics.js';

const DAY_MS = 864e5;
const WEEKDAYS = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];

// ------------------------------------------------------------------ config
function hexRgb(hex) {
  const h = hex.replace('#', '');
  const n = parseInt(h.length === 3 ? h.replace(/./g, '$&$&') : h, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
function slug(s) { return String(s).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, ''); }

const CFG = (function () {
  const tz = RAW.timezone || 'UTC';
  const people = (RAW.people || []).map((p) => {
    const [r, g, b] = hexRgb(p.color || '#888888');
    const light = (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.62;
    // notify: a Home Assistant notify service (notify.mobile_app_…) for notes addressed to them.
    return { id: p.id, name: p.name, color: p.color, rgb: r + ' ' + g + ' ' + b, ink: light ? '#111111' : '#FFFFFF', ha: p.ha,
      notify: /^notify\.\w+$/.test(p.notify || '') ? p.notify : null };
  });
  const shop = RAW.shopping || {};
  const cats = shop.categories || ['Other'];
  const stores = shop.stores && shop.stores.length ? shop.stores : [{ id: 'grocer', name: 'Groceries' }];
  const catOrder = { all: cats };
  stores.forEach((s) => { catOrder[s.id] = (shop.storeOrder && shop.storeOrder[s.id]) || cats; });
  const catalog = [];
  Object.keys(shop.catalog || {}).forEach((cat) => shop.catalog[cat].forEach((name) => catalog.push([name, cat])));

  const home = RAW.home && RAW.home.url ? RAW.home : null;
  const rooms = [], devices = [];
  if (home) (home.rooms || []).forEach((r) => {
    const id = r.id || slug(r.name);
    rooms.push({ id, name: r.name });
    (r.devices || []).forEach((d) => devices.push({ entity: d.entity, name: d.name, room: id, garage: !!d.garage }));
  });
  // "off": true on the doorbell turns it off (no alert, no camera takeover) and keeps its settings.
  const alerts = Object.assign({}, (home && home.alerts) || {});
  if (alerts.doorbell && alerts.doorbell.off) delete alerts.doorbell;
  const bins = RAW.bins && RAW.bins.every && RAW.bins.every.length ? RAW.bins : null;
  const warnings = RAW.warnings && RAW.warnings.region ? RAW.warnings : null;
  const display = RAW.display || {};
  const frames = RAW.frames || {};
  return {
    tz, people, bins, warnings, display,
    // Notify services (e.g. "notify.mobile_app_pixel") told when a frame goes quiet or comes back.
    frameNotify: home && Array.isArray(frames.notify) ? frames.notify.filter((s) => /^notify\.\w+$/.test(s)) : [],
    frameQuietMinutes: Math.max(20, frames.quietMinutes || 20),
    location: RAW.location, stores, catOrder, catalog, usuals: shop.usuals || [],
    home, rooms, devices,
    scenes: home ? (home.scenes || []).map((s) => ({ id: s.entity, name: s.name, icon: s.icon || 'bolt' })) : [],
    cameras: home ? (home.cameras || []).map((c) => ({ id: c.entity, name: c.name })) : [],
    alerts,
  };
})();

// ------------------------------------------------------------------ helpers
function json(data, status) {
  return new Response(JSON.stringify(data), { status: status || 200, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } });
}
function bad(msg, status) { return json({ error: msg }, status || 400); }
function str(v, max) { return typeof v === 'string' ? v.trim().slice(0, max || 200) : ''; }
async function body(req) { try { return await req.json(); } catch (e) { return {}; } }

// Local calendar day in the family's time zone: { ymd: '2026-10-16', dow: 5 } (0 = Sunday)
function localDay(ms) {
  const parts = new Intl.DateTimeFormat('en-CA', { timeZone: CFG.tz, year: 'numeric', month: '2-digit', day: '2-digit', weekday: 'short' })
    .formatToParts(new Date(ms == null ? Date.now() : ms));
  const get = (t) => parts.find((p) => p.type === t).value;
  return { ymd: get('year') + '-' + get('month') + '-' + get('day'), dow: WEEKDAYS.indexOf(get('weekday').slice(0, 3).toLowerCase()) };
}

async function cached(env, key, ttlMs, compute) {
  const row = await env.DB.prepare('SELECT value, expires FROM cache WHERE key = ?').bind(key).first();
  if (row && row.expires > Date.now()) return JSON.parse(row.value);
  try {
    const value = await compute();
    await env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
      .bind(key, JSON.stringify(value), Date.now() + ttlMs).run();
    return value;
  } catch (err) {
    if (row) return JSON.parse(row.value); // serve stale rather than nothing
    throw err;
  }
}

// ------------------------------------------------------------------ auth
const enc = new TextEncoder();
function sameKey(a, b) {
  if (!a || !b) return false;
  const x = enc.encode(a), y = enc.encode(b);
  return x.byteLength === y.byteLength && crypto.subtle.timingSafeEqual(x, y);
}
function cookieKey(req) {
  const m = /(?:^|;\s*)hq=([^;]+)/.exec(req.headers.get('cookie') || '');
  return m ? decodeURIComponent(m[1]) : null;
}

// ------------------------------------------------------------------ bins
// Collection dates come from the council's rotation in settings.json ("every"), with bank-holiday
// changes listed in "moved" (usual date -> actual date, or null when it is skipped).
function addYmd(ymd, n) {
  const d = new Date(ymd + 'T12:00:00Z');
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
}
async function binsState(env) {
  const B = CFG.bins;
  if (!B) return null;
  const today = localDay().ymd, until = addYmd(today, 42), moved = B.moved || {}, by = {};
  B.every.forEach((x) => {
    const step = 7 * (x.weeks || 1);
    let d = x.from;
    if (d < today) d = addYmd(d, Math.ceil((Date.parse(today) - Date.parse(d)) / 864e5 / step) * step - step);
    for (; d <= until; d = addYmd(d, step)) {
      const actual = Object.prototype.hasOwnProperty.call(moved, d) ? moved[d] : d;
      if (!actual || actual < today) continue;
      (by[actual] = by[actual] || []).push(x.bin);
    }
  });
  const next = Object.keys(by).sort().slice(0, 4).map((date) => ({ date, bins: by[date] }));
  const out = await env.DB.prepare('SELECT value FROM cache WHERE key = ?').bind('bins-out').first();
  return { types: B.types || {}, next, out: out ? JSON.parse(out.value) : null };
}
// "They're out" for one collection date; out: false takes it back.
async function setBinsOut(env, b) {
  const date = /^\d{4}-\d{2}-\d{2}$/.test(b.date) ? b.date : null;
  if (!date) return bad('date required');
  if (b.out === false) await env.DB.prepare('DELETE FROM cache WHERE key = ?').bind('bins-out').run();
  else await env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
    .bind('bins-out', JSON.stringify({ date, at: Date.now() }), Date.now() + 7 * 864e5).run();
  return json(await binsState(env));
}

// ------------------------------------------------------------------ frames
// Each wall display checks in every 5 minutes; the list shows when one has gone quiet.
// A frame's id lives in its browser storage, which belongs to one address, so after a move to a
// new address (or cleared storage) it checks in as a new frame. Its old entry, same name and quiet
// for 15 minutes, is removed then. Entries silent for 30 days expire on their own.
async function frameBeat(env, b) {
  const id = str(b.id, 40);
  if (!/^[a-z0-9]{6,40}$/.test(id)) return bad('id required');
  const row = {
    id, name: str(b.name, 40) || 'Frame', version: str(b.version, 20), build: str(b.build, 40), kiosk: str(b.kiosk, 20),
    rssi: typeof b.rssi === 'number' ? Math.round(b.rssi) : null, model: str(b.model, 60), android: str(b.android, 10),
    loaded: typeof b.loaded === 'number' ? b.loaded : null, seen: Date.now(),
  };
  await env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
    .bind('frame:' + id, JSON.stringify(row), Date.now() + 30 * 864e5).run();
  const stale = (await listFrames(env)).filter((f) => f.id !== id && f.name === row.name && row.seen - f.seen >= 15 * 60e3);
  if (stale.length) await env.DB.batch(stale.map((f) => env.DB.prepare('DELETE FROM cache WHERE key = ?').bind('frame:' + f.id)));
  return json({ ok: true, replaced: stale.length });
}
async function listFrames(env) {
  const rows = await env.DB.prepare("SELECT value FROM cache WHERE key LIKE 'frame:%' AND expires > ?").bind(Date.now()).all();
  return rows.results.map((r) => JSON.parse(r.value)).sort((a, b) => (a.name < b.name ? -1 : 1));
}

// Every 5 minutes (cron in wrangler.jsonc): tell Home Assistant's notify services when a frame has
// been quiet for frames.quietMinutes (20 at least, so a frame that just moved address is tidied
// up first, see frameBeat) and again when it's back. Both use one Android tag per frame, so
// "back" replaces "offline" in the notification shade. Sent once per outage.
function clock(ms) {
  return new Intl.DateTimeFormat('en-GB', { timeZone: CFG.tz, hour: '2-digit', minute: '2-digit' }).format(new Date(ms));
}
function duration(ms) {
  const m = Math.round(ms / 60000);
  return m < 60 ? m + ' min' : Math.floor(m / 60) + ' h' + (m % 60 ? ' ' + (m % 60) + ' min' : '');
}
async function notify(env, title, message, tag) {
  await Promise.all(CFG.frameNotify.map((s) => haService(env, 'notify', s.split('.')[1], { title, message, data: { tag, group: 'homehq' } })));
}
async function checkFrames(env) {
  if (!CFG.frameNotify.length || !env.HA_TOKEN) return;
  const now = Date.now(), frames = await listFrames(env);
  const row = await env.DB.prepare('SELECT value FROM cache WHERE key = ?').bind('frame-alerts').first();
  const alerted = row ? JSON.parse(row.value) : {}; // frame id -> when it was last heard from before going quiet
  let changed = false;
  for (const f of frames) {
    const quiet = now - f.seen >= CFG.frameQuietMinutes * 60e3;
    if (quiet && !alerted[f.id]) {
      await notify(env, f.name + ' frame is offline', 'Last checked in at ' + clock(f.seen) + '. Check it’s switched on and on Wi-Fi.', 'homehq-frame-' + f.id);
      alerted[f.id] = f.seen;
      changed = true;
    } else if (!quiet && alerted[f.id]) {
      await notify(env, f.name + ' frame is back', 'Offline for about ' + duration(now - alerted[f.id]) + '.', 'homehq-frame-' + f.id);
      delete alerted[f.id];
      changed = true;
    }
  }
  // Forget frames that left the list (renamed address, expired) without a "back" message.
  Object.keys(alerted).forEach((id) => { if (!frames.some((f) => f.id === id)) { delete alerted[id]; changed = true; } });
  if (changed) await env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
    .bind('frame-alerts', JSON.stringify(alerted), now + 365 * 864e5).run();
}

// ------------------------------------------------------------------ pushed alerts
// Polling Home Assistant can't be both light and quick, so for the urgent things (doorbell, smoke
// and CO, leaks) a Home Assistant automation calls POST /api/alert the moment one changes. That
// bumps a counter, clears the shared snapshot, and tells the AlertHub Durable Object, which sends the
// alert down the WebSocket every frame keeps open (/api/alerts/ws): the frame hears about it
// within a moment and refreshes Home at once, so its usual alert logic sees the change. Nothing
// polls, so waiting costs no Worker CPU (a held-open request that re-checked D1 cost 22 ms).
// The counter in D1 is the fallback for a frame whose socket is down (GET /api/alerts).
const ALERT_TYPES = ['doorbell', 'smoke', 'leak', 'test'];
async function pushAlert(req, env, url) {
  const given = url.searchParams.get('t') || ((req.headers.get('authorization') || '').replace(/^Bearer\s+/i, ''));
  if (!env.ALERT_TOKEN || !given || !sameKey(given, env.ALERT_TOKEN)) return bad('locked', 401);
  const b = await body(req), type = ALERT_TYPES.indexOf(b.type) > -1 ? b.type : 'test';
  const row = await env.DB.prepare('SELECT value FROM cache WHERE key = ?').bind('alert-seq').first();
  const seq = (row ? JSON.parse(row.value).seq : 0) + 1;
  await env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
    .bind('alert-seq', JSON.stringify({ seq, type, at: Date.now() }), Date.now() + 365 * DAY_MS).run();
  await forgetSnapshot(env);
  await caches.default.delete(new Request('https://cache.homehq.internal/ha-fail')).catch(() => {});
  await alertHub(env, '/bump', { method: 'POST', body: JSON.stringify({ seq, type, at: Date.now() }) }).catch((err) => console.error('alert hub', err));
  return json({ ok: true, seq });
}
async function alertSeq(env) {
  const row = await env.DB.prepare('SELECT value FROM cache WHERE key = ?').bind('alert-seq').first();
  return row ? JSON.parse(row.value) : { seq: 0 };
}
// The one AlertHub instance (a Durable Object): it holds every frame's WebSocket. It must be one
// object so a push, handled by any run of the Worker anywhere, reaches all of them.
function alertHub(env, path, init) {
  if (!env.ALERTS) return Promise.reject(new Error('no ALERTS binding'));
  return env.ALERTS.get(env.ALERTS.idFromName('hub')).fetch('https://alert-hub' + path, init);
}
// GET /api/alerts/ws: a frame's WebSocket, handed to the hub.
function alertSocket(req, env) {
  if (req.headers.get('upgrade') !== 'websocket') return bad('expected a WebSocket', 426);
  return alertHub(env, '/socket', { headers: req.headers });
}
// Hibernates between messages: while idle it costs nothing, and the frame's 30 s "ping" is answered
// by the platform without waking it. A new connection is sent the latest alert, so the frame knows
// the current counter; a bump is sent to every open connection.
export class AlertHub {
  constructor(ctx) {
    this.ctx = ctx;
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping', 'pong'));
  }
  async fetch(req) {
    const path = new URL(req.url).pathname;
    if (path === '/bump') {
      const msg = await req.text();
      await this.ctx.storage.put('last', msg);
      this.ctx.getWebSockets().forEach((ws) => { try { ws.send(msg); } catch (e) { /* closing */ } });
      return new Response('ok');
    }
    const pair = new WebSocketPair();
    this.ctx.acceptWebSocket(pair[1]);
    pair[1].send((await this.ctx.storage.get('last')) || '{"seq":0}');
    return new Response(null, { status: 101, webSocket: pair[0] });
  }
  webSocketMessage() { /* pings are answered automatically; nothing else is expected */ }
  webSocketClose(ws, code) { try { ws.close(code); } catch (e) { /* already closed */ } }
}

// ------------------------------------------------------------------ sticky notes
// Short notes for the household, written on a frame or the phone page and shown on every frame
// until cleared or expired. A note for one person also goes to their phone (people[].notify).
const NOTE_MAX = 140;
async function liveNotes(env) {
  const rows = await env.DB.prepare('SELECT id, text, from_id, to_id, created, expires FROM notes WHERE cleared IS NULL AND (expires IS NULL OR expires > ?) ORDER BY created DESC LIMIT 20')
    .bind(Date.now()).all();
  return rows.results.map((n) => ({ id: n.id, text: n.text, from: n.from_id, to: n.to_id, created: n.created, expires: n.expires }));
}
async function putNote(env, b) {
  const id = str(b.id, 40), text = str(b.text, NOTE_MAX);
  const person = (pid) => CFG.people.find((p) => p.id === pid);
  const to = b.to === 'all' || person(b.to) ? b.to : 'all';
  const expires = typeof b.expires === 'number' && b.expires > Date.now() ? b.expires : null;
  if (!/^[a-z0-9]{6,40}$/.test(id) || !text || !person(b.from)) return bad('id, text and from (a person) required');
  const old = await env.DB.prepare('SELECT id FROM notes WHERE id = ?').bind(id).first();
  await env.DB.prepare('INSERT INTO notes (id, text, from_id, to_id, created, expires, cleared) VALUES (?, ?, ?, ?, ?, ?, NULL) ' +
    'ON CONFLICT(id) DO UPDATE SET text = excluded.text, to_id = excluded.to_id, expires = excluded.expires')
    .bind(id, text, b.from, to, Date.now(), expires).run();
  // A new note for one person: tell their phone too. Best effort; the note is saved either way.
  const target = !old && to !== 'all' && person(to), from = person(b.from);
  if (target && target.notify && CFG.home && env.HA_TOKEN) {
    await haService(env, 'notify', target.notify.split('.')[1], { title: '📌 Note from ' + from.name, message: text, data: { tag: 'homehq-note-' + id, group: 'homehq' } })
      .catch((err) => console.warn('note push', err.message));
  }
  return json({ ok: true, notes: await liveNotes(env) });
}
async function clearNote(env, id, undo) {
  await env.DB.prepare('UPDATE notes SET cleared = ? WHERE id = ?').bind(undo ? null : Date.now(), id).run();
  return json({ ok: true, notes: await liveNotes(env) });
}

// ------------------------------------------------------------------ state (shopping, photos, bins)
async function getState(env) {
  const [groceries, photos, bins, notes] = await Promise.all([
    env.DB.prepare('SELECT id, name, cat, qty, store, note, done FROM groceries WHERE cleared = 0 ORDER BY updated_at').all(),
    env.DB.prepare('SELECT id, o, date, place, from_id, fav FROM photos ORDER BY date DESC').all(),
    binsState(env),
    liveNotes(env),
  ]);
  return {
    people: CFG.people.map((p) => ({ id: p.id, name: p.name, color: p.color, rgb: p.rgb, ink: p.ink, tracked: !!(p.ha && CFG.home) })),
    stores: CFG.stores, catOrder: CFG.catOrder, catalog: CFG.catalog, usuals: CFG.usuals,
    groceries: groceries.results.map((g) => Object.assign(g, { done: !!g.done })),
    photos: photos.results.map((p) => ({ id: p.id, o: p.o, date: p.date, place: p.place, from: p.from_id, fav: !!p.fav, tag: p.date })),
    hasHome: !!CFG.home,
    scenes: CFG.scenes, cameras: CFG.cameras,
    bins, notes,
    // Minutes everyone known to Home Assistant must be away before the screen turns off (0 = never).
    awayOffMinutes: CFG.display.awayOffMinutes || 0,
  };
}

async function putGrocery(env, b) {
  const id = str(b.id, 64), name = str(b.name, 80);
  if (!id || !name) return bad('id and name required');
  const cat = str(b.cat, 40) || 'Other';
  const store = CFG.stores.some((s) => s.id === b.store) ? b.store : CFG.stores[0].id;
  const qty = Math.max(1, Math.min(99, parseInt(b.qty, 10) || 1));
  await env.DB.prepare(
    'INSERT INTO groceries (id, name, cat, qty, store, note, done, cleared, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?) ' +
    'ON CONFLICT(id) DO UPDATE SET name = excluded.name, cat = excluded.cat, qty = excluded.qty, store = excluded.store, note = excluded.note, done = excluded.done, cleared = 0, updated_at = excluded.updated_at'
  ).bind(id, name, cat, qty, store, str(b.note, 120), b.done ? 1 : 0, Date.now()).run();
  return json({ ok: true });
}

async function clearGroceries(env, b) {
  const ids = Array.isArray(b.ids) ? b.ids.filter((x) => typeof x === 'string').slice(0, 500) : [];
  if (!ids.length) return json({ ok: true });
  const now = Date.now();
  await env.DB.batch(ids.map((id) => env.DB.prepare('UPDATE groceries SET cleared = ?, updated_at = ? WHERE id = ?').bind(b.cleared === false ? 0 : 1, now, id))
    .concat([env.DB.prepare('DELETE FROM groceries WHERE cleared = 1 AND updated_at < ?').bind(now - 30 * DAY_MS)]));
  return json({ ok: true });
}

// ------------------------------------------------------------------ calendar
// The calendar links are private (anyone holding one can read that calendar), so they live in the
// CALENDARS secret (from secrets.json via `npm run secrets`), which Cloudflare encrypts and never
// shows again, not in the deployed code. Locally, `npm run secrets -- --local` puts it in .dev.vars.
function calendarList(env) {
  if (!env.CALENDARS) return [];
  try { return JSON.parse(env.CALENDARS); } catch (e) { throw new Error('CALENDARS secret is not valid JSON'); }
}

const CAL_TTL_MS = 10 * 60e3;

// Fetch each calendar on its own. One bad link must not hide the others: a calendar that fails keeps
// the events it had last time and its error is reported (never its link) in the log, in the
// response and on the frame's Status panel.
async function refreshCalendars(env, prev) {
  const from = Date.now() - 45 * DAY_MS, to = Date.now() + 75 * DAY_MS;
  const all = CFG.people.map((p) => p.id);
  let list;
  try {
    list = calendarList(env);
    if (!Array.isArray(list)) throw new Error('CALENDARS must be a list [ … ]');
  } catch (e) {
    console.error('calendars: ' + e.message);
    return { events: prev ? prev.events : [], errors: [e.message], checked: Date.now() };
  }
  const results = await Promise.all(list.map(async (cal, i) => {
    const n = i + 1, notes = [];
    try {
      if (!cal || typeof cal.url !== 'string') throw new Error('no "url"');
      const wanted = [].concat(cal.people || []).indexOf('all') > -1 ? all : [].concat(cal.people || []);
      const unknown = wanted.filter((id) => all.indexOf(id) < 0);
      if (unknown.length) notes.push('Calendar ' + n + ': unknown people ' + unknown.join(', ') + ' (ids must match settings.json)');
      const people = wanted.filter((id) => all.indexOf(id) > -1);
      const res = await fetch(cal.url.trim().replace(/^webcal:/, 'https:'), { headers: { 'user-agent': 'HomeHQ/1.0' } });
      if (!res.ok) throw new Error('HTTP ' + res.status + (res.status === 404 ? ' (link wrong, or reset in Google)' : ''));
      const text = await res.text();
      if (text.slice(0, 2000).indexOf('BEGIN:VCALENDAR') < 0) throw new Error('not an iCal feed (use "Secret address in iCal format")');
      const events = parseIcs(text, people.length ? people : all, from, to, 'c' + i + '-');
      console.log('calendar ' + n + ': ' + events.length + ' events');
      return { events, notes };
    } catch (e) {
      console.error('calendar ' + n + ': ' + e.message);
      const kept = prev ? prev.events.filter((ev) => ev.id.indexOf('c' + i + '-') === 0) : [];
      return { events: kept, notes: notes.concat(['Calendar ' + n + ': ' + e.message]) };
    }
  }));
  // The same invite on two people's calendars becomes one event with both of them.
  const byKey = new Map();
  results.forEach((r) => r.events.forEach((e) => {
    const k = e.key || e.id, prev = byKey.get(k);
    if (prev) e.people.forEach((p) => { if (prev.people.indexOf(p) < 0) prev.people.push(p); });
    else byKey.set(k, Object.assign({}, e, { people: e.people.slice() }));
  }));
  return {
    events: Array.from(byKey.values()), // keep key (UID|start): getCalendar matches frame events by it
    errors: results.reduce((a, r) => a.concat(r.notes), []),
    checked: Date.now(),
  };
}

// ------------------------------------------------------------------ live drive times
// For events with a location starting in the next 4 hours: Google's traffic-aware drive time from
// wherever Home Assistant places the event's people (their phone's position), or from home
// (zone.home) for anyone it can't place. With several people, the longest drive counts. Kept 10
// minutes per event, failures too, so a place Google can't find isn't asked again each poll.
// Needs GOOGLE_MAPS_KEY (secrets.json); without it, or when Google can't answer, Travel: is used.
// At most DRIVE_DAILY_MAX requests to Google a day (counted here, whatever the Cloud console
// allows); after that, Travel: until midnight.
const DRIVE_AHEAD_MS = 4 * 3600e3, DRIVE_TTL_MS = 10 * 60e3, DRIVE_MAX_MIN = 180, DRIVE_DAILY_MAX = 300;
async function driveBudget(env) {
  const key = 'drive-count:' + localDay().ymd;
  const row = await env.DB.prepare('SELECT value FROM cache WHERE key = ?').bind(key).first();
  let used = row ? +row.value : 0;
  return {
    take() { if (used >= DRIVE_DAILY_MAX) return false; used++; return true; },
    save() {
      return env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
        .bind(key, String(used), Date.now() + 2 * DAY_MS).run();
    },
  };
}
async function homeStates(env) {
  if (!CFG.home || !env.HA_TOKEN) return {};
  try {
    const by = {};
    (await haSnapshot(env)).forEach((s) => { by[s.entity_id] = s; });
    return by;
  } catch (e) { return {}; }
}
function where(s) {
  const a = (s && s.attributes) || {};
  return typeof a.latitude === 'number' && typeof a.longitude === 'number' ? { latitude: a.latitude, longitude: a.longitude } : null;
}
async function driveMinutes(env, origin, address) {
  const res = await fetch('https://routes.googleapis.com/directions/v2:computeRoutes', {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'X-Goog-Api-Key': env.GOOGLE_MAPS_KEY, 'X-Goog-FieldMask': 'routes.duration' },
    body: JSON.stringify({ origin: { location: { latLng: origin } }, destination: { address }, travelMode: 'DRIVE', routingPreference: 'TRAFFIC_AWARE', regionCode: 'gb' }),
  });
  if (!res.ok) { console.warn('Routes API: HTTP ' + res.status); return null; }
  const r = ((await res.json()).routes || [])[0];
  const secs = r && parseInt(r.duration, 10);
  return secs > 0 ? Math.ceil(secs / 60) : null;
}
async function driveFor(env, ev, states, budget) {
  const home = where(states['zone.home']);
  const starts = {};
  ev.people.forEach((id) => {
    const p = CFG.people.find((x) => x.id === id);
    const at = p && p.ha ? where(states[p.ha]) : null;
    if (at) starts[p.name] = at; else if (home) starts.home = home;
  });
  let best = null;
  for (const from of Object.keys(starts).slice(0, 3)) {
    if (!budget.take()) break;
    const mins = await driveMinutes(env, starts[from], ev.loc);
    if (mins != null && mins <= DRIVE_MAX_MIN && (!best || mins > best.mins)) best = { mins, from: from === 'home' ? 'home' : from };
  }
  return best;
}
async function addDrives(env, events) {
  if (!env.GOOGLE_MAPS_KEY) return;
  const now = Date.now();
  const due = events.filter((e) => e.loc && !e.allDay && Date.parse(e.start) > now && Date.parse(e.start) - now < DRIVE_AHEAD_MS).slice(0, 8);
  if (!due.length) return;
  const row = await env.DB.prepare('SELECT value FROM cache WHERE key = ?').bind('drives').first();
  const saved = row ? JSON.parse(row.value) : {}, kept = {};
  let states = null, asked = false, budget = null;
  for (const e of due) {
    let d = saved[e.id];
    if (!d || d.loc !== e.loc || now - d.at > DRIVE_TTL_MS) {
      states = states || await homeStates(env);
      budget = budget || await driveBudget(env);
      d = Object.assign({ loc: e.loc, at: now }, (await driveFor(env, e, states, budget).catch(() => null)) || { mins: null });
      asked = true;
    }
    kept[e.id] = d;
    if (d.mins != null) e.drive = { mins: d.mins, from: d.from };
  }
  if (asked) await env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
    .bind('drives', JSON.stringify(kept), now + DAY_MS).run();
  if (budget) await budget.save();
}

async function getCalendar(env) {
  const row = await env.DB.prepare("SELECT value, expires FROM cache WHERE key = 'calendar'").first();
  let feed = row ? JSON.parse(row.value) : null;
  if (Array.isArray(feed)) feed = { events: feed, errors: [], checked: 0 }; // saved by an older version
  if (!feed || row.expires <= Date.now()) {
    feed = await refreshCalendars(env, feed);
    await env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
      .bind('calendar', JSON.stringify(feed), Date.now() + CAL_TTL_MS).run();
  }
  const local = await env.DB.prepare('SELECT id, title, people, start, end, gid FROM events WHERE end > ?').bind(Date.now() - 45 * DAY_MS).all();
  // A frame event written to Google comes back in the iCal feed as UID <gid>@google.com; from then
  // on the feed's copy is shown and this one is hidden.
  const inFeed = (gid) => feed.events.some((f) => f.key && f.key.indexOf(gid + '@google.com|') === 0);
  const events = feed.events.map((e) => { const c = Object.assign({}, e); delete c.key; return c; }).concat(local.results.filter((e) => !(e.gid && inFeed(e.gid))).map((e) => ({
    id: e.id, title: e.title, people: JSON.parse(e.people), start: new Date(e.start).toISOString(), end: new Date(e.end).toISOString(), local: true,
    google: !!e.gid,
  })));
  await addDrives(env, events).catch((err) => console.error('addDrives', err));
  return { events, errors: feed.errors, checked: feed.checked };
}

// ------------------------------------------------------------------ writing to Google Calendar
// Events added on the frame are also written to Google with a service account (secrets.json
// GOOGLE_SERVICE_ACCOUNT, the JSON key file) that each calendar is shared with ("Make changes to
// events"). Which calendar: the CALENDARS entry for exactly these people ("all" for everyone),
// else the "all" one. Its id is the part of the secret iCal link after /ical/. The frame keeps its
// own copy until the iCal feed carries the event, then hides it (see getCalendar).
let googleAuth = null; // { token, exp }, reused while this Worker instance lives
function b64url(bytes) {
  let bin = '';
  new Uint8Array(bytes).forEach((c) => { bin += String.fromCharCode(c); });
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
async function googleToken(env) {
  if (googleAuth && googleAuth.exp > Date.now() + 60e3) return googleAuth.token;
  const sa = JSON.parse(env.GOOGLE_SERVICE_ACCOUNT), now = Math.floor(Date.now() / 1000);
  const part = (o) => b64url(enc.encode(JSON.stringify(o)));
  const unsigned = part({ alg: 'RS256', typ: 'JWT' }) + '.' + part({
    iss: sa.client_email, scope: 'https://www.googleapis.com/auth/calendar.events',
    aud: 'https://oauth2.googleapis.com/token', iat: now, exp: now + 3600,
  });
  const der = Uint8Array.from(atob(sa.private_key.replace(/-----[^-]+-----/g, '').replace(/\s+/g, '')), (c) => c.charCodeAt(0));
  const key = await crypto.subtle.importKey('pkcs8', der, { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' }, false, ['sign']);
  const jwt = unsigned + '.' + b64url(await crypto.subtle.sign('RSASSA-PKCS1-v1_5', key, enc.encode(unsigned)));
  const res = await fetch('https://oauth2.googleapis.com/token', {
    method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: 'grant_type=' + encodeURIComponent('urn:ietf:params:oauth:grant-type:jwt-bearer') + '&assertion=' + jwt,
  });
  const j = await res.json().catch(() => ({}));
  if (!res.ok || !j.access_token) throw new Error('Google sign-in failed (' + (j.error_description || j.error || 'HTTP ' + res.status) + ')');
  googleAuth = { token: j.access_token, exp: Date.now() + (j.expires_in || 3600) * 1000 };
  return googleAuth.token;
}
function calendarFor(env, people) {
  const list = calendarList(env).map((c) => {
    const m = /\/ical\/([^/]+)\//.exec(c.url || '');
    return { id: m ? decodeURIComponent(m[1]) : null, people: c.people === 'all' ? 'all' : [].concat(c.people || []) };
  }).filter((c) => c.id);
  const everyone = people.length >= CFG.people.length;
  const exact = list.find((c) => (c.people === 'all' ? everyone : c.people.length === people.length && people.every((p) => c.people.indexOf(p) > -1)));
  return (exact || list.find((c) => c.people === 'all') || {}).id || null;
}
async function google(env, method, path, body) {
  const res = await fetch('https://www.googleapis.com/calendar/v3' + path, {
    method, headers: { authorization: 'Bearer ' + await googleToken(env), 'content-type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (res.status === 204 || res.status === 404 || res.status === 410) return {}; // deleted, or gone already
  const j = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error((j.error && j.error.message) || 'HTTP ' + res.status);
  return j;
}

async function addEvent(env, b) {
  const id = str(b.id, 64), title = str(b.title, 120);
  const start = +b.start, end = +b.end;
  const people = (Array.isArray(b.people) ? b.people : []).filter((id) => CFG.people.some((p) => p.id === id));
  if (!id || !title || !(end > start) || !people.length) return bad('id, title, people, start < end required');
  await env.DB.prepare('INSERT OR REPLACE INTO events (id, title, people, start, end, created_at) VALUES (?, ?, ?, ?, ?, ?)')
    .bind(id, title, JSON.stringify(people), start, end, Date.now()).run();
  if (!env.GOOGLE_SERVICE_ACCOUNT) return json({ ok: true });
  const cal = calendarFor(env, people);
  if (!cal) return json({ ok: true, google: false, error: 'no calendar for these people in CALENDARS' });
  try {
    const g = await google(env, 'POST', '/calendars/' + encodeURIComponent(cal) + '/events', {
      summary: title,
      start: { dateTime: new Date(start).toISOString(), timeZone: CFG.tz },
      end: { dateTime: new Date(end).toISOString(), timeZone: CFG.tz },
      description: 'Added on the Home HQ frame',
    });
    await env.DB.prepare('UPDATE events SET gcal = ?, gid = ? WHERE id = ?').bind(cal, g.id, id).run();
    return json({ ok: true, google: true });
  } catch (err) {
    return json({ ok: true, google: false, error: String(err.message || err) });
  }
}
async function deleteEvent(env, id) {
  const row = await env.DB.prepare('SELECT gcal, gid FROM events WHERE id = ?').bind(id).first();
  if (row && row.gid && env.GOOGLE_SERVICE_ACCOUNT) {
    try { await google(env, 'DELETE', '/calendars/' + encodeURIComponent(row.gcal) + '/events/' + encodeURIComponent(row.gid)); } catch (err) {
      return bad('Couldn’t remove it from Google Calendar: ' + (err.message || err), 502); // keep it here so it can be tried again
    }
  }
  await env.DB.prepare('DELETE FROM events WHERE id = ?').bind(id).run();
  return json({ ok: true });
}

// ------------------------------------------------------------------ weather (Open-Meteo, no key)
function wxCond(code, isDay) {
  if (code <= 1) return isDay ? 'sun' : 'moon';
  if (code === 2) return 'partly';
  if (code === 3 || code === 45 || code === 48) return 'cloud';
  if ((code >= 71 && code <= 77) || code === 85 || code === 86) return 'snow';
  return 'rain';
}
async function getWeather(env) {
  const [w, warnings] = await Promise.all([
    forecast(env).catch((err) => ({ error: err.message })),
    getWarnings(env).catch(() => []),
  ]);
  return Object.assign({}, w, { warnings });
}
async function forecast(env) {
  const loc = CFG.location;
  if (!loc) return { none: true };
  return cached(env, 'weather', 15 * 60e3, async () => {
    const q = new URLSearchParams({
      latitude: loc.lat, longitude: loc.lon, timezone: CFG.tz, forecast_days: '2',
      temperature_unit: loc.units === 'fahrenheit' ? 'fahrenheit' : 'celsius',
      wind_speed_unit: loc.units === 'fahrenheit' ? 'mph' : 'kmh',
      hourly: 'temperature_2m,precipitation_probability,weather_code,is_day',
      daily: 'temperature_2m_max,temperature_2m_min',
      current: 'temperature_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m',
    });
    const res = await fetch('https://api.open-meteo.com/v1/forecast?' + q);
    if (!res.ok) throw new Error('weather: HTTP ' + res.status);
    const w = await res.json(), h = w.hourly;
    return {
      day: h.time[0].slice(0, 10), // index 0 = local midnight today; 48 hours
      units: loc.units === 'fahrenheit' ? 'fahrenheit' : 'celsius',
      t: h.temperature_2m.map(Math.round),
      p: h.precipitation_probability.map((x) => x || 0),
      c: h.weather_code.map((code, i) => wxCond(code, h.is_day[i])),
      hi: Math.round(w.daily.temperature_2m_max[0]), lo: Math.round(w.daily.temperature_2m_min[0]),
      feels: Math.round(w.current.apparent_temperature), humidity: w.current.relative_humidity_2m,
      wind: Math.round(w.current.wind_speed_10m), now: Math.round(w.current.temperature_2m),
    };
  });
}

// Met Office warnings for one region (settings.json "warnings": { "region": "yh", "areas": [...] }),
// kept to those naming one of the areas. Times are UK local, as 'YYYY-MM-DDTHH:MM'.
const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];
function xmlText(s) {
  return s.replace(/<!\[CDATA\[([\s\S]*?)\]\]>/g, '$1').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&').trim();
}
function warnTime(hhmm, day, mon, year) {
  return year + '-' + String(MONTHS.indexOf(mon.toLowerCase()) + 1).padStart(2, '0') + '-' + day.padStart(2, '0') + 'T' + hhmm.slice(0, 2) + ':' + hhmm.slice(2);
}
async function getWarnings(env) {
  const W = CFG.warnings;
  if (!W) return [];
  return cached(env, 'warnings', 10 * 60e3, async () => {
    const res = await fetch('https://www.metoffice.gov.uk/public/data/PWSCache/WarningsRSS/Region/' + encodeURIComponent(W.region));
    if (!res.ok) throw new Error('warnings: HTTP ' + res.status);
    const xml = await res.text(), want = (W.areas || []).map((a) => a.toLowerCase()), seen = {}, out = [];
    (xml.match(/<item>[\s\S]*?<\/item>/g) || []).forEach((item) => {
      const tag = (t) => { const m = new RegExp('<' + t + '[^>]*>([\\s\\S]*?)</' + t + '>').exec(item); return m ? xmlText(m[1]) : ''; };
      const d = /^(Yellow|Amber|Red) warning of (.+?) affecting (.+?):\s*(.*?)\s+valid from (\d{4}) \w{3} (\d{1,2}) (\w{3}) to (\d{4}) \w{3} (\d{1,2}) (\w{3})/i.exec(tag('description'));
      if (!d) return;
      const areas = d[4].split(/,\s*/), mine = want.length ? areas.filter((a) => want.indexOf(a.toLowerCase()) > -1) : areas;
      if (!mine.length) return;
      const link = tag('link'), y = +((/date=(\d{4})/.exec(link) || [])[1] || new Date().getUTCFullYear());
      const from = warnTime(d[5], d[6], d[7], y);
      const to = warnTime(d[8], d[9], d[10], MONTHS.indexOf(d[10].toLowerCase()) < MONTHS.indexOf(d[7].toLowerCase()) ? y + 1 : y);
      const level = d[1].toLowerCase(), type = d[2].toLowerCase(), key = level + type + from + to;
      if (seen[key]) return;
      seen[key] = true;
      out.push({ level, type, region: d[3], areas: mine, from, to, link });
    });
    const rank = { red: 0, amber: 1, yellow: 2 };
    return out.sort((a, b) => rank[a.level] - rank[b.level] || (a.from < b.from ? -1 : 1));
  });
}

// ------------------------------------------------------------------ Home Assistant
// Every call gives up after 15 s (history 30 s): when Home Assistant is struggling, frames should
// show it offline, not stack up requests that make it worse.
function ha(env, path, init) {
  init = init || {};
  init.headers = Object.assign({ authorization: 'Bearer ' + env.HA_TOKEN, 'content-type': 'application/json' }, init.headers || {});
  if (!init.signal) init.signal = AbortSignal.timeout(path.indexOf('/api/history') === 0 ? 30e3 : 15e3);
  return fetch(CFG.home.url.replace(/\/$/, '') + path, init);
}

// ------------------------------------------------------------------ reading Home Assistant
// One GET of /api/states (every entity, ~800 KB here; a non-admin user can't render templates),
// reduced to the few dozen the dashboard shows plus the media players and the alert sensors'
// batteries it finds in the same list. The reduced result is shared by every frame for 8 s through
// Cloudflare's edge cache, so frames, phones and the energy panel cost one read between them;
// controlling something clears it. (1.13 read the entities one at a time, to spare Home Assistant;
// 1.20 went back to the single read because the ~36 subrequests cost more Worker CPU than the free
// plan's 10 ms allows, and Home Assistant copes with it now.)
const SNAP_TTL = 8;
async function edgeCache(key, ttl, compute) {
  const req = new Request('https://cache.homehq.internal/' + key);
  const hit = await caches.default.match(req);
  if (hit) return hit.json();
  const value = await compute();
  await caches.default.put(req, new Response(JSON.stringify(value), { headers: { 'content-type': 'application/json', 'cache-control': 'max-age=' + ttl } }));
  return value;
}
// Something is about to be controlled (or an alert fired): drop this location's shared copies, and mark
// the pushed state out of date until Home Assistant's next push, so the next reads go to Home Assistant
// itself and show the change at once. Call this BEFORE the control, not after: Home Assistant pushes the
// new state moments after it acts, possibly before the Worker's call to it returns, and a push is only
// accepted if it's newer than this mark. After the control, dropSnapshots() clears anything a read
// meanwhile put back, without touching the mark.
function forgetSnapshot(env) {
  const drop = [dropSnapshots()];
  if (env) drop.push(putCache(env, 'ha-dirty', { at: Date.now() }, DAY_MS));
  return Promise.all(drop).catch(() => {});
}
// The control didn't happen (refused, or Home Assistant failed): lift the mark so pushes are used again
// instead of every read going to Home Assistant until the next push.
function unforgetSnapshot(env) {
  return env.DB.prepare('DELETE FROM cache WHERE key = ?').bind('ha-dirty').run().catch(() => {});
}
function dropSnapshots() {
  return Promise.all(['ha-snap', 'home-view'].map((k) => caches.default.delete(new Request('https://cache.homehq.internal/' + k)))).catch(() => {});
}
function coreIds() {
  const H = CFG.home, al = CFG.alerts, E = H.energy || {}, ids = new Set(['zone.home']);
  CFG.devices.forEach((d) => ids.add(d.entity));
  [H.climate, al.doorbell && al.doorbell.entity, al.doorbell && al.doorbell.lock, al.leak && al.leak.valve, E.power]
    .forEach((id) => { if (id) ids.add(id); });
  [].concat((al.leak && al.leak.entities) || [], (al.smoke && al.smoke.entities) || [], E.cost || [], E.usage || []).forEach((id) => ids.add(id));
  CFG.people.forEach((p) => { if (p.ha) ids.add(p.ha); });
  return ids;
}
// Every 5 minutes: which media players exist (one per name, the most active) and which battery
// sensors belong to the alert devices. Small lists, kept in the edge cache.
// The last good lists are also kept in D1: when the full list fails (Home Assistant busy or
// restarting) the small per-entity reads carry on with them instead of failing too.
// The states the dashboard needs, as /api/states would give them, but only those.
// A failure is remembered for 30 s: while Home Assistant is down or swamped, frames get "offline"
// straight away instead of every poll asking it again.
async function haSnapshot(env) {
  const failReq = new Request('https://cache.homehq.internal/ha-fail');
  const failed = await caches.default.match(failReq);
  if (failed) throw new Error(await failed.text());
  try {
    // The list of states, with where it came from ('push' or 'read') and when, as properties.
    const snap = await haSnapshotFetch(env);
    return Object.assign(snap.states, { via: snap.via, at: snap.at });
  } catch (err) {
    await caches.default.put(failReq, new Response(err.message || 'Home Assistant unreachable', { headers: { 'cache-control': 'max-age=30' } }));
    throw err;
  }
}
// Home Assistant pushes the states the dashboard shows to POST /api/ha-push (its automation, see
// `npm run ha-state`), on every change and once a minute. They're kept in D1 and read here: a few
// KB to parse instead of Home Assistant's whole 800 KB list, which cost most of the Worker's CPU.
// Used while it's recent and no control has happened since; otherwise the Worker reads Home
// Assistant itself, as before, so a broken automation only costs CPU, never the data.
const PUSH_FRESH_MS = 150e3;
async function putCache(env, key, value, ttlMs) {
  await env.DB.prepare('INSERT INTO cache (key, value, expires) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, expires = excluded.expires')
    .bind(key, JSON.stringify(value), Date.now() + ttlMs).run();
}
// What to keep from a list of states: what's shown, every media player worth listing, and the alert
// sensors' batteries.
function reduceStates(all) {
  const keep = coreIds();
  playersView(CFG.home, all).forEach((p) => keep.add(p.id));
  const al = CFG.alerts, watched = [].concat((al.leak && al.leak.entities) || [], (al.smoke && al.smoke.entities) || []).map((id) => id.split('.')[1]);
  all.forEach((x) => {
    if (x.entity_id.indexOf('sensor.') === 0 && (x.attributes || {}).device_class === 'battery' &&
        watched.some((w) => w.indexOf(x.entity_id.slice(7).replace(/_battery(_level)?$/, '')) === 0)) keep.add(x.entity_id);
  });
  return all.filter((x) => keep.has(x.entity_id));
}
async function pushState(req, env, url) {
  const given = url.searchParams.get('t') || ((req.headers.get('authorization') || '').replace(/^Bearer\s+/i, ''));
  if (!env.ALERT_TOKEN || !given || !sameKey(given, env.ALERT_TOKEN)) return bad('locked', 401);
  const text = await req.text();
  if (text.length > 2e6) return bad('too large', 413);
  let b;
  try { b = JSON.parse(text); } catch (e) { return bad('not JSON', 400); }
  const list = Array.isArray(b && b.states) ? b.states.filter((x) => x && typeof x.entity_id === 'string' && x.state != null) : [];
  if (!list.length) return bad('no states', 400);
  const states = reduceStates(list);
  await putCache(env, 'ha-push', { at: Date.now(), states }, DAY_MS);
  return json({ ok: true, kept: states.length, of: list.length });
}
async function pushedStates(env) {
  const rows = await env.DB.prepare("SELECT key, value FROM cache WHERE key IN ('ha-push', 'ha-dirty')").all();
  let push = null, dirty = 0;
  rows.results.forEach((r) => { if (r.key === 'ha-push') push = JSON.parse(r.value); else dirty = (JSON.parse(r.value) || {}).at || 0; });
  return push && Date.now() - push.at < PUSH_FRESH_MS && push.at > dirty ? push : null;
}
async function haSnapshotFetch(env) {
  return edgeCache('ha-snap', SNAP_TTL, async () => {
    const pushed = await pushedStates(env).catch(() => null);
    if (pushed) return { via: 'push', at: pushed.at, states: pushed.states };
    const res = await ha(env, '/api/states');
    if (!res.ok) throw new Error('Home Assistant: HTTP ' + res.status);
    const all = await res.json();
    if (!Array.isArray(all) || !all.length) throw new Error('Home Assistant unreachable');
    return { via: 'read', at: Date.now(), states: reduceStates(all) };
  });
}
async function haService(env, domain, service, data) {
  const res = await ha(env, '/api/services/' + domain + '/' + service, { method: 'POST', body: JSON.stringify(data) });
  if (!res.ok) throw new Error(domain + '.' + service + ': HTTP ' + res.status);
}

function deviceView(d, s) {
  const domain = d.entity.split('.')[0], a = (s && s.attributes) || {};
  const out = { id: d.entity, room: d.room, name: d.name || a.friendly_name || d.entity, offline: !s || s.state === 'unavailable' };
  const state = s ? s.state : 'unavailable';
  if (domain === 'lock') return Object.assign(out, { type: 'lock', locked: state === 'locked' });
  if (domain === 'cover' && (d.garage || a.device_class === 'garage' || a.device_class === 'gate')) {
    return Object.assign(out, { type: 'garage', state: ['open', 'opening', 'closing'].indexOf(state) > -1 ? state : 'closed' });
  }
  if (domain === 'cover') return Object.assign(out, { type: 'blind', pos: a.current_position != null ? a.current_position : state === 'open' ? 100 : 0 });
  const dims = domain === 'light' && (a.brightness != null || (a.supported_color_modes || []).some((m) => m !== 'onoff'));
  return Object.assign(out, { type: 'light', on: state === 'on', level: a.brightness ? Math.max(1, Math.round(a.brightness / 2.55)) : 100, dims: !!dims });
}

async function getHome(env) {
  if (!CFG.home) return { none: true };
  if (!env.HA_TOKEN) return { online: false, error: 'HA_TOKEN secret is not set' };
  let states;
  try {
    states = await haSnapshot(env);
  } catch (err) {
    return { online: false, error: /HTTP/.test(err.message) ? err.message : 'Home Assistant unreachable' };
  }
  const by = {};
  states.forEach((s) => { by[s.entity_id] = s; });
  const H = CFG.home, al = CFG.alerts;
  const c = H.climate && by[H.climate];
  const ca = c ? c.attributes : {};
  const devices = CFG.devices.map((d) => deviceView(d, by[d.entity]));
  const garage = devices.find((d) => d.type === 'garage');
  const doorbell = al.doorbell && by[al.doorbell.entity];
  return {
    online: true,
    rooms: CFG.rooms, devices, scenes: CFG.scenes, cameras: CFG.cameras,
    climate: c ? {
      current: ca.current_temperature, set: ca.temperature != null ? ca.temperature : ca.target_temp_low,
      mode: c.state === 'heat_cool' ? 'auto' : c.state, modes: ca.hvac_modes || [],
      humidity: ca.current_humidity, step: ca.target_temp_step || 0.5, min: ca.min_temp || 7, max: ca.max_temp || 35,
    } : null,
    presence: CFG.people.filter((p) => p.ha).map((p) => {
      const s = by[p.ha];
      return { id: p.id, home: !!s && s.state === 'home', status: !s ? 'Unknown' : s.state === 'home' ? 'Home' : s.state === 'not_home' ? 'Away' : s.state };
    }),
    sensors: {
      doorbell: doorbell ? { state: doorbell.state, changed: doorbell.last_changed } : null,
      leak: ((al.leak && al.leak.entities) || []).some((e) => by[e] && by[e].state === 'on'),
      // Smoke and CO detectors that are going off right now, by name, so the alert can say where.
      smoke: ((al.smoke && al.smoke.entities) || []).filter((e) => by[e] && by[e].state === 'on').map((e) => {
        const a = by[e].attributes || {};
        return { name: a.friendly_name || e, co: a.device_class === 'carbon_monoxide' || /(^|[._])co(_|$)|carbon_monoxide/.test(e) };
      }),
      garageOpenSince: garage && garage.state !== 'closed' && by[garage.id] ? by[garage.id].last_changed : null,
    },
    energy: energyView(H.energy, by),
    players: playersView(H, states),
    checks: checksView(al, by),
    via: states.via, viaAt: states.at,
    alertConfig: {
      doorbell: !!al.doorbell, doorbellCamera: al.doorbell && al.doorbell.camera || null, doorbellLock: al.doorbell && al.doorbell.lock || null,
      waterValve: !!(al.leak && al.leak.valve), smoke: !!(al.smoke && al.smoke.entities && al.smoke.entities.length),
      garageOpenMinutes: al.garageOpenMinutes || 0,
    },
  };
}

// ------------------------------------------------------------------ media players
// Every media_player Home Assistant has, for the Media tab. Integrations often register one device
// several times (Cast and Assistant SDK, old TV entries), so players are merged by name, keeping
// the most active; unavailable ones are left out until they come back. home.players: { "exclude":
// [entity ids], "names": { entity id: name } } tidies the list.
const MEDIA_RANK = { playing: 0, paused: 1, buffering: 1, idle: 2, on: 3, standby: 4, off: 5 };
function mediaAllowed(H, id) {
  const P = H.players || {};
  return /^media_player\.\w+$/.test(id) && (P.exclude || []).indexOf(id) < 0;
}
function playersView(H, states) {
  const P = H.players || {}, best = {};
  states.forEach((s) => {
    if (s.entity_id.indexOf('media_player.') !== 0 || !mediaAllowed(H, s.entity_id) || !(s.state in MEDIA_RANK)) return;
    const a = s.attributes || {}, name = (P.names || {})[s.entity_id] || a.friendly_name || s.entity_id;
    const prev = best[name];
    if (prev && MEDIA_RANK[prev.state] <= MEDIA_RANK[s.state]) return;
    best[name] = {
      id: s.entity_id, name, state: s.state,
      title: a.media_title || '', artist: a.media_artist || a.media_album_name || '', app: a.app_name || '',
      // Something to resume: playing, paused, or idle with media still loaded. An idle player with
      // nothing loaded (a Cast or Music Assistant speaker after it finishes) can't play, skip or go back.
      loaded: s.state === 'playing' || s.state === 'paused' || !!(a.media_content_id || a.media_title),
      volume: typeof a.volume_level === 'number' ? Math.round(a.volume_level * 100) : null, muted: !!a.is_volume_muted,
      art: a.entity_picture ? '/api/ha/art?id=' + encodeURIComponent(s.entity_id) + '&v=' + encodeURIComponent(a.entity_picture.slice(-24)) : null,
      features: a.supported_features || 0,
    };
  });
  return Object.values(best).sort((x, y) => MEDIA_RANK[x.state] - MEDIA_RANK[y.state] || (x.name < y.name ? -1 : 1));
}
const MEDIA_DO = {
  play_pause: ['media_play_pause'], next: ['media_next_track'], previous: ['media_previous_track'],
  on: ['turn_on'], off: ['turn_off'],
  volume: ['volume_set', (v) => ({ volume_level: Math.max(0, Math.min(100, +v || 0)) / 100 })],
  mute: ['volume_mute', (v) => ({ is_volume_muted: !!v })],
};
async function mediaControl(env, id, b) {
  const op = MEDIA_DO[b.do];
  if (!op || !mediaAllowed(CFG.home, id)) return bad('unknown player or action', 404);
  await forgetSnapshot(env);
  try {
    await haService(env, 'media_player', op[0], Object.assign({ entity_id: id }, op[1] ? op[1](b.value) : {}));
  } catch (err) {
    await unforgetSnapshot(env);
    // Home Assistant answers 500 when the player can't do it (nothing loaded, or it's asleep).
    const s = await ha(env, '/api/states/' + id).then((r) => r.ok ? r.json() : null).catch(() => null);
    const name = (s && s.attributes && s.attributes.friendly_name) || id;
    const why = /^(play_pause|next|previous)$/.test(b.do) ? 'nothing to play or resume' : 'it didn’t accept that';
    return bad(name + ': ' + why, 409);
  }
  await dropSnapshots();
  return json({ ok: true });
}

// The "all clear" line: sensors behind the alerts that have gone quiet, the water main, and low
// batteries on those same devices (a sensor.<name>_battery whose <name> starts the alert entity's).
function checksView(al, by) {
  const off = (id) => !by[id] || by[id].state === 'unavailable' || by[id].state === 'unknown';
  const name = (id) => (by[id] && by[id].attributes.friendly_name) || id;
  const leaks = (al.leak && al.leak.entities) || [], smoke = (al.smoke && al.smoke.entities) || [];
  const watched = leaks.concat(smoke).map((id) => id.split('.')[1]);
  const lowBattery = Object.keys(by).filter((id) => {
    const s = by[id], a = s.attributes || {};
    if (id.indexOf('sensor.') !== 0 || a.device_class !== 'battery') return false;
    const stem = id.slice(7).replace(/_battery(_level)?$/, '');
    const pct = parseFloat(s.state);
    return !isNaN(pct) && pct < 20 && watched.some((w) => w.indexOf(stem) === 0);
  }).map((id) => ({ name: name(id).replace(/\s*battery( level)?$/i, ''), pct: Math.round(parseFloat(by[id].state)) }));
  const v = al.leak && al.leak.valve && by[al.leak.valve];
  return {
    leaksOffline: leaks.filter(off).map(name), leakTotal: leaks.length,
    smokeOffline: smoke.filter(off).map(name), smokeTotal: smoke.length,
    // A valve.* reports open/closed; a switch.* standing in for one is on while the water is on.
    valveClosed: v ? (v.state === 'closed' || v.state === 'off') : null,
    lowBattery,
  };
}

// settings.json home.energy: { "cost": "sensor.…" or a list to add up (money today, e.g. electricity
// and gas), "power": "sensor.…" (W or kW now), "high": 3000 (watts that count as a lot) }.
// Either can be left out.
// Today so far, for the energy panel: electricity per hour, worked out from the power sensor's
// history (Home Assistant records a reading each time it changes, so a reading holds until the
// next). Also the same point yesterday, today's peak, and when the smart meter's own usage
// sensors (home.energy.usage) last changed: data collected via the DCC often arrives hours or a
// day late, so the panel says so instead of showing stale numbers as today's. Optional
// home.energy.rate (£/kWh) and standing (£/day) give an estimated cost. Kept 5 minutes.
// Hour buckets are midnight + n hours, so on the two clock-change days one hour is off by one.
function localMidnight(ms) {
  const parts = new Intl.DateTimeFormat('en-GB', { timeZone: CFG.tz, hour: 'numeric', minute: 'numeric', second: 'numeric', hourCycle: 'h23' }).formatToParts(new Date(ms));
  const get = (t) => +parts.find((x) => x.type === t).value;
  return ms - ((get('hour') * 60 + get('minute')) * 60 + get('second')) * 1000 - (ms % 1000);
}
function wattHours(points, from, to, unitK) {
  // points: [[ms, watts|NaN]] sorted; returns Wh between from and to, per hour from `from`.
  const hours = new Array(Math.ceil((to - from) / 36e5)).fill(0);
  points.forEach((pt, i) => {
    const w = unitK ? pt[1] * 1000 : pt[1];
    if (!(w >= 0)) return; // unavailable or unknown: no reading for that stretch
    let a = Math.max(pt[0], from);
    const b = Math.min(i + 1 < points.length ? points[i + 1][0] : to, to);
    while (a < b) {
      const h = Math.floor((a - from) / 36e5), edge = Math.min(b, from + (h + 1) * 36e5);
      hours[h] += w * (edge - a) / 36e5;
      a = edge;
    }
  });
  return hours;
}
async function powerHistory(env, from, to) {
  const E = CFG.home.energy;
  const r = await ha(env, '/api/history/period/' + new Date(from).toISOString() + '?minimal_response&no_attributes&end_time=' + encodeURIComponent(new Date(to).toISOString()) + '&filter_entity_id=' + E.power);
  if (!r.ok) throw new Error('history: HTTP ' + r.status);
  const list = (await r.json())[0] || [];
  return list.map((x) => [Date.parse(x.last_changed || x.last_updated), parseFloat(x.state)]);
}
async function energyToday(env) {
  const E = CFG.home && CFG.home.energy;
  if (!E || !E.power) return { none: true };
  const now = Date.now(), midnight = localMidnight(now);
  const by = {};
  (await haSnapshot(env).catch(() => [])).forEach((x) => { by[x.entity_id] = x; });
  const cur = by[E.power], unitK = cur && /^kw$/i.test((cur.attributes || {}).unit_of_measurement || '');
  // Yesterday's 24 hourly totals: once a day. Today's: every 15 minutes. Both are recorder queries.
  const yday = await cached(env, 'energy-yday:' + midnight, DAY_MS + 36e5, async () =>
    wattHours(await powerHistory(env, midnight - DAY_MS, midnight), midnight - DAY_MS, midnight, unitK).map((wh) => wh / 1000));
  const today = await cached(env, 'energy-today', 15 * 60e3, async () => {
    const pts = await powerHistory(env, midnight, now);
    let peak = null;
    pts.forEach((pt) => { const w = unitK ? pt[1] * 1000 : pt[1]; if (w >= 0 && (!peak || w > peak.w)) peak = { w: Math.round(w), at: pt[0] }; });
    return { midnight, until: now, hours: wattHours(pts, midnight, now, unitK).map((wh) => +(wh / 1000).toFixed(3)), peak };
  });
  if (today.midnight !== midnight) return { none: true }; // just past midnight, before the next refresh
  const total = today.hours.reduce((t, k) => t + k, 0);
  // Yesterday by the same time: whole hours, then the share of the hour in progress.
  const h = Math.floor((now - midnight) / 36e5), part = ((now - midnight) % 36e5) / 36e5;
  const yesterdayByNow = yday.slice(0, h).reduce((t, k) => t + k, 0) + (yday[h] || 0) * part;
  const nowW = cur ? parseFloat(cur.state) * (unitK ? 1000 : 1) : null;
  const reported = [].concat(E.usage || []).map((id) => by[id] && parseFloat(by[id].state) > 0 ? Date.parse(by[id].last_changed) : 0);
  return {
    midnight, now: today.until, hours: today.hours, total: +total.toFixed(2), yesterdayByNow: +yesterdayByNow.toFixed(2),
    nowW: nowW >= 0 ? Math.round(nowW) : null, peak: today.peak,
    cost: E.rate ? +(total * E.rate + (E.standing || 0)).toFixed(2) : null,
    meterReported: reported.length ? Math.max.apply(null, reported) || null : null,
  };
}

function energyView(E, by) {
  if (!E) return null;
  const num = (id) => { const s = id && by[id]; const n = s ? parseFloat(s.state) : NaN; return isNaN(n) ? null : { n, unit: (s.attributes || {}).unit_of_measurement || '' }; };
  const costs = [].concat(E.cost || []).map(num).filter(Boolean), power = num(E.power);
  const cost = costs.length ? { n: costs.reduce((t, c) => t + c.n, 0), unit: costs[0].unit } : null;
  const watts = power ? (/^kw$/i.test(power.unit) ? power.n * 1000 : power.n) : null;
  return {
    cost: cost ? cost.n : null, currency: cost ? ({ GBP: '£', EUR: '€', USD: '$' }[cost.unit] || cost.unit || '£') : null,
    watts, high: watts != null && watts >= (E.high || 3000),
  };
}

async function controlDevice(env, entity, p) {
  const d = CFG.devices.find((x) => x.entity === entity) || (CFG.alerts.doorbell && CFG.alerts.doorbell.lock === entity ? { entity } : null);
  if (!d) return bad('unknown device', 404);
  const domain = entity.split('.')[0], target = { entity_id: entity };
  const known = (domain === 'lock' && 'locked' in p) || (domain === 'cover' && ('state' in p || 'pos' in p)) || 'on' in p;
  if (!known) return bad('nothing to change');
  await forgetSnapshot(env);
  try {
  if (domain === 'lock' && 'locked' in p) await haService(env, 'lock', p.locked ? 'lock' : 'unlock', target);
  else if (domain === 'cover' && 'state' in p) await haService(env, 'cover', p.state === 'open' || p.state === 'opening' ? 'open_cover' : 'close_cover', target);
  else if (domain === 'cover' && 'pos' in p) {
    const pos = Math.max(0, Math.min(100, Math.round(+p.pos)));
    if (pos === 0) await haService(env, 'cover', 'close_cover', target);
    else if (pos === 100) await haService(env, 'cover', 'open_cover', target);
    else await haService(env, 'cover', 'set_cover_position', Object.assign({ position: pos }, target));
  } else if ('on' in p) {
    if (!p.on) await haService(env, domain, 'turn_off', target);
    else if (domain === 'light' && p.level) await haService(env, 'light', 'turn_on', Object.assign({ brightness_pct: Math.max(1, Math.min(100, Math.round(+p.level))) }, target));
    else await haService(env, domain, 'turn_on', target);
  }
  } catch (err) {
    await unforgetSnapshot(env);
    throw err;
  }
  await dropSnapshots();
  return json({ ok: true });
}

async function homeAction(env, what, b) {
  await forgetSnapshot(env);
  let res;
  try { res = await homeActionRun(env, what, b); } catch (err) { await unforgetSnapshot(env); throw err; }
  if (res.ok) await dropSnapshots(); else await unforgetSnapshot(env);
  return res;
}
async function homeActionRun(env, what, b) {
  const H = CFG.home;
  if (what === 'climate' && H.climate) {
    if (b.mode) await haService(env, 'climate', 'set_hvac_mode', { entity_id: H.climate, hvac_mode: b.mode === 'auto' ? 'heat_cool' : str(b.mode, 20) });
    if (b.set != null) await haService(env, 'climate', 'set_temperature', { entity_id: H.climate, temperature: +b.set });
    return json({ ok: true });
  }
  if (what === 'scene' && CFG.scenes.some((s) => s.id === b.id)) { await haService(env, 'scene', 'turn_on', { entity_id: b.id }); return json({ ok: true }); }
  if (what === 'water-off' && CFG.alerts.leak && CFG.alerts.leak.valve) {
    const v = CFG.alerts.leak.valve, domain = v.split('.')[0];
    await haService(env, domain, domain === 'valve' ? 'close_valve' : 'turn_off', { entity_id: v });
    return json({ ok: true });
  }
  return bad('unknown action', 404);
}

// A camera snapshot, with the last good one kept (at this location's edge cache, for 6 h) and sent
// instead when Home Assistant fails or is slow: a camera that's asleep or busy makes Home Assistant
// answer "500" after its own 10 s timeout, which left the frame with a broken-image icon every 20 s.
// The stale copy says how old it is in x-stale (seconds).
const CAM_KEEP_S = 6 * 3600;
async function cameraImage(env, id) {
  const key = new Request('https://cache.homehq.internal/cam/' + encodeURIComponent(id));
  const old = async () => {
    const hit = await caches.default.match(key);
    if (!hit) return null;
    const saved = +hit.headers.get('x-saved') || Date.now();
    return new Response(hit.body, { headers: Object.assign(IMAGE_HEADERS(hit.headers.get('content-type') || 'image/jpeg'), { 'x-stale': String(Math.round((Date.now() - saved) / 1000)) }) });
  };
  try {
    const res = await ha(env, '/api/camera_proxy/' + id, { headers: { 'content-type': '' }, signal: AbortSignal.timeout(8e3) });
    const type = res.headers.get('content-type') || '';
    if (res.ok && /^image\//.test(type)) {
      const buf = await res.arrayBuffer();
      await caches.default.put(key, new Response(buf, { headers: { 'content-type': type, 'x-saved': String(Date.now()), 'cache-control': 'max-age=' + CAM_KEEP_S } })).catch(() => {});
      return new Response(buf, { headers: IMAGE_HEADERS(type) });
    }
    if (res.body) res.body.cancel().catch(() => {});
    return (await old()) || bad('Home Assistant: camera HTTP ' + res.status, 502);
  } catch (err) {
    return (await old()) || bad('camera unavailable', 504);
  }
}
// Pictures fetched for the frame come back as plain images only: whatever the source claims, it is
// passed on as an image type with nosniff and a sandboxing CSP, so even a page at that address can't
// run under the dashboard's origin.
const IMAGE_HEADERS = (type, cache) => ({ 'content-type': type, 'cache-control': cache || 'no-store', 'x-content-type-options': 'nosniff', 'content-security-policy': "default-src 'none'; sandbox" });
// Artwork's address changes with the track (&v=), so the browser may keep it a while: no Worker call per render.
const ART_CACHE = 'private, max-age=600';
async function imageResponse(res, cache) {
  const type = res.headers.get('content-type') || '';
  if (!res.ok || !/^image\/[a-z0-9.+-]+$/i.test(type.split(';')[0].trim())) {
    if (res.body) res.body.cancel().catch(() => {});
    return new Response(null, { status: 404 });
  }
  return new Response(res.body, { status: 200, headers: IMAGE_HEADERS(type.split(';')[0].trim(), cache) });
}
async function haImage(env, which, url) {
  if (which === 'camera') {
    const id = url.searchParams.get('id');
    const ok = CFG.cameras.some((c) => c.id === id) || (CFG.alerts.doorbell && CFG.alerts.doorbell.camera === id);
    if (!ok) return bad('unknown camera', 404);
    return cameraImage(env, id);
  }
  const id = url.searchParams.get('id');
  if (!id || !mediaAllowed(CFG.home, id)) return bad('no media player', 404);
  const s = await ha(env, '/api/states/' + id);
  const a = s.ok ? (await s.json()).attributes || {} : {};
  // Music Assistant and similar give a LAN address in entity_picture; Home Assistant also offers the
  // same picture through itself as entity_picture_local, which the Worker can reach.
  const pic = a.entity_picture_local || a.entity_picture;
  if (!pic) return new Response(null, { status: 404 });
  if (/^[a-z][a-z0-9+.-]*:/i.test(pic)) {
    // An absolute address: https only (a LAN or plain-http address can't be reached from Cloudflare, and
    // isn't worth the risk), with a time limit.
    let u;
    try { u = new URL(pic); } catch (e) { return new Response(null, { status: 404 }); }
    if (u.protocol !== 'https:') return new Response(null, { status: 404 });
    return imageResponse(await fetch(u.href, { signal: AbortSignal.timeout(8e3) }).catch(() => new Response(null, { status: 502 })), ART_CACHE);
  }
  return imageResponse(await ha(env, pic, { headers: { 'content-type': '' } }), ART_CACHE);
}

// ------------------------------------------------------------------ photos
async function uploadPhoto(env, req) {
  const form = await req.formData();
  const full = form.get('full'), small = form.get('small');
  let meta = {};
  try { meta = JSON.parse(form.get('meta') || '{}'); } catch (e) {}
  if (!full || !small || typeof full === 'string') return bad('full and small images required');
  if (full.size > 12e6 || small.size > 4e6) return bad('image too large', 413);
  const id = crypto.randomUUID().replace(/-/g, '').slice(0, 16);
  const date = /^\d{4}-\d{2}-\d{2}$/.test(meta.date) ? meta.date : localDay().ymd;
  const from = CFG.people.some((p) => p.id === meta.from) ? meta.from : null;
  const opts = { httpMetadata: { contentType: 'image/jpeg' } };
  await Promise.all([env.PHOTOS.put('full/' + id + '.jpg', full.stream(), opts), env.PHOTOS.put('small/' + id + '.jpg', small.stream(), opts)]);
  await env.DB.prepare('INSERT INTO photos (id, o, date, place, from_id, fav, created_at) VALUES (?, ?, ?, ?, ?, 0, ?)')
    .bind(id, meta.o === 'p' ? 'p' : 'l', date, str(meta.place, 80), from, Date.now()).run();
  return json({ id });
}

async function servePhoto(env, id, small) {
  const obj = await env.PHOTOS.get((small ? 'small/' : 'full/') + id + '.jpg');
  if (!obj) return new Response('Not found', { status: 404 });
  return new Response(obj.body, { headers: { 'content-type': 'image/jpeg', 'cache-control': 'private, max-age=31536000, immutable', etag: obj.httpEtag } });
}

// ------------------------------------------------------------------ router
async function api(req, env, url) {
  const p = url.pathname.replace(/^\/api/, ''), m = req.method;
  let r;
  if (p === '/state' && m === 'GET') return json(await getState(env));
  if (p === '/calendar' && m === 'GET') return json(await getCalendar(env));
  if (p === '/weather' && m === 'GET') return json(await getWeather(env));
  if (p === '/events' && m === 'POST') return addEvent(env, await body(req));
  if ((r = /^\/events\/([\w-]+)$/.exec(p)) && m === 'DELETE') return deleteEvent(env, r[1]);
  if (p === '/bins/out' && m === 'POST') return setBinsOut(env, await body(req));
  if (p === '/frames' && m === 'POST') return frameBeat(env, await body(req));
  if (p === '/frames' && m === 'GET') return json(await listFrames(env));
  // Display controls test: one notification to every frames.notify service.
  if (p === '/frames/test-notify' && m === 'POST') {
    if (!CFG.frameNotify.length) return bad('Add frames.notify to settings.json first', 404);
    await notify(env, 'Test from Home HQ', 'Frame notifications reach this device.', 'homehq-frame-test');
    return json({ ok: true, sent: CFG.frameNotify.length });
  }
  if ((r = /^\/frames\/([a-z0-9]{6,40})$/.exec(p)) && m === 'DELETE') {
    await env.DB.prepare('DELETE FROM cache WHERE key = ?').bind('frame:' + r[1]).run();
    return json({ ok: true });
  }
  if (p === '/alerts/ws' && m === 'GET') return alertSocket(req, env);
  if (p === '/alerts' && m === 'GET') {
    // The counter, for a frame whose socket is down. A page from before the socket (1.16-1.21)
    // sends ?since= and loops straight back on every answer: hold it for 20 s (one sleep, no
    // database checks) until it has reloaded onto the new page, instead of letting it spin.
    const a = await alertSeq(env), since = url.searchParams.get('since');
    if (since !== null && since !== '' && +since === a.seq) {
      await new Promise((r) => setTimeout(r, 20e3));
      return json(await alertSeq(env));
    }
    return json(a);
  }
  if (p === '/notes' && m === 'POST') return putNote(env, await body(req));
  if ((r = /^\/notes\/([a-z0-9]{6,40})$/.exec(p)) && m === 'DELETE') return clearNote(env, r[1], false);
  if ((r = /^\/notes\/([a-z0-9]{6,40})\/restore$/.exec(p)) && m === 'POST') return clearNote(env, r[1], true);
  if (p === '/groceries' && m === 'POST') return putGrocery(env, await body(req));
  if (p === '/groceries/clear' && m === 'POST') return clearGroceries(env, await body(req));

  if (p === '/photos' && m === 'POST') return uploadPhoto(env, req);
  if ((r = /^\/photos\/(\w+)$/.exec(p))) {
    if (m === 'GET') return servePhoto(env, r[1], url.searchParams.has('s'));
    if (m === 'PATCH') {
      await env.DB.prepare('UPDATE photos SET fav = ? WHERE id = ?').bind((await body(req)).fav ? 1 : 0, r[1]).run();
      return json({ ok: true });
    }
    if (m === 'DELETE') {
      await env.PHOTOS.delete(['full/' + r[1] + '.jpg', 'small/' + r[1] + '.jpg']);
      await env.DB.prepare('DELETE FROM photos WHERE id = ?').bind(r[1]).run();
      return json({ ok: true });
    }
  }

  if (p.indexOf('/home') === 0 || p.indexOf('/ha/') === 0) {
    if (!CFG.home) return bad('Home Assistant is not configured', 404);
    if (!env.HA_TOKEN) return bad('HA_TOKEN secret is not set', 503);
    // The finished view is shared for 8 s too (frames, phones), on top of the shared states read.
    if (p === '/home' && m === 'GET') return json(await edgeCache('home-view', SNAP_TTL, () => getHome(env)));
    if (p === '/home/energy' && m === 'GET') return json(await energyToday(env));
    if ((r = /^\/home\/device\/([\w.]+)$/.exec(p)) && m === 'POST') return controlDevice(env, r[1], await body(req));
    if ((r = /^\/home\/media\/(media_player\.\w+)$/.exec(p)) && m === 'POST') return mediaControl(env, r[1], await body(req));
    if ((r = /^\/home\/([\w-]+)$/.exec(p)) && m === 'POST') return homeAction(env, r[1], await body(req));
    if (p === '/ha/camera' || p === '/ha/art') return haImage(env, p.slice(4), url);
  }
  return bad('not found', 404);
}

export default {
  async scheduled(event, env, ctx) {
    ctx.waitUntil(checkFrames(env).catch((err) => console.error('checkFrames', err)));
  },
  async fetch(req, env) {
    const url = new URL(req.url);
    if (!env.DASHBOARD_KEY) return new Response('Set the DASHBOARD_KEY secret first: npx wrangler secret put DASHBOARD_KEY', { status: 500 });

    // ?key=… on any page: remember it in a cookie and drop it from the address bar.
    const given = url.searchParams.get('key');
    if (given != null && url.pathname.indexOf('/api/') !== 0) {
      if (!sameKey(given, env.DASHBOARD_KEY)) return new Response('Wrong key', { status: 403 });
      url.searchParams.delete('key');
      return new Response(null, {
        status: 302,
        headers: {
          location: url.pathname + url.search + url.hash,
          'set-cookie': 'hq=' + encodeURIComponent(given) + '; Path=/; Max-Age=315360000; HttpOnly; Secure; SameSite=Lax',
        },
      });
    }
    // A share that arrived before the phone's service worker was running: the photos can't be
    // kept here (the upload needs the key), so open the upload page to pick them again.
    if (url.pathname === '/share' && req.method === 'POST') return Response.redirect(url.origin + '/upload?shared=0', 303);
    // Home Assistant pushes urgent alerts here with its own token (ALERT_TOKEN), never the dashboard key.
    if (url.pathname === '/api/alert' && req.method === 'POST') return pushAlert(req, env, url);
    // ...and the states the dashboard shows (see pushState), with the same token.
    if (url.pathname === '/api/ha-push' && req.method === 'POST') return pushState(req, env, url);
    if (url.pathname.indexOf('/api/') !== 0) return env.ASSETS.fetch(req);
    if (!sameKey(cookieKey(req) || req.headers.get('x-hq-key'), env.DASHBOARD_KEY)) return bad('locked', 401);
    try {
      return await api(req, env, url);
    } catch (err) {
      console.error(err);
      return bad(String(err.message || err), 502);
    }
  },
};
