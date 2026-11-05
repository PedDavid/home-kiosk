// Upload secrets.json to the Worker as Cloudflare secrets: DASHBOARD_KEY, HA_TOKEN, CALENDARS and
// GOOGLE_MAPS_KEY (optional: live drive times for events with a location) and
// GOOGLE_SERVICE_ACCOUNT (optional: events added on the frame are written to Google Calendar) and
// ALERT_TOKEN (optional: lets Home Assistant push urgent alerts; see `npm run ha-alerts`).
// Cloudflare never shows a secret again, so secrets.json is your only readable copy: keep it out
// of git (refused below) and out of synced folders, and back it up somewhere safe.
//
//   npm run secrets             check, then upload every secret that has a value
//   npm run secrets -- --check  check only
//   npm run secrets -- --local  write .dev.vars for `wrangler dev` instead of uploading
//
// Prints secret names, never values. A blank value is skipped, so it never replaces a secret
// already on Cloudflare.
import { readFileSync, writeFileSync, chmodSync, existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const dir = new URL('..', import.meta.url);
const file = new URL('secrets.json', dir);
const fail = (msg) => { console.error('secrets.json: ' + msg); process.exit(1); };
const mode = process.argv.includes('--check') ? 'check' : process.argv.includes('--local') ? 'local' : 'upload';

if (!existsSync(file)) fail('not found. Copy secrets.example.json to secrets.json and fill it in.');

// Never upload from a file git could commit.
const git = (args) => spawnSync('git', args, { cwd: dir, stdio: 'ignore' }).status;
if (git(['ls-files', '--error-unmatch', 'secrets.json']) === 0) fail('is tracked by git. Run `git rm --cached secrets.json` and commit that first.');
if (git(['rev-parse']) === 0 && git(['check-ignore', '-q', 'secrets.json']) !== 0) fail('is not in .gitignore. Add it before going on.');
chmodSync(file, 0o600); // readable by you only

const raw = readFileSync(file, 'utf8');
let S;
try { S = JSON.parse(raw); } catch (e) {
  const m = /position (\d+)/.exec(e.message);
  fail('not valid JSON near line ' + (m ? raw.slice(0, +m[1]).split('\n').length : '?') + '. Check the quotes and commas around it.');
}

// ------------------------------------------------------------ checks
const key = typeof S.DASHBOARD_KEY === 'string' ? S.DASHBOARD_KEY.trim() : '';
if (key && key.length < 12) fail('DASHBOARD_KEY is short; use at least 12 random characters.');
const token = typeof S.HA_TOKEN === 'string' ? S.HA_TOKEN.trim() : '';
if (token && token.split('.').length !== 3) fail('HA_TOKEN doesn’t look like a Home Assistant long-lived token (three parts separated by dots).');

const maps = typeof S.GOOGLE_MAPS_KEY === 'string' ? S.GOOGLE_MAPS_KEY.trim() : '';
if (maps && !/^AIza[\w-]{30,}$/.test(maps)) fail('GOOGLE_MAPS_KEY doesn’t look like a Google API key (starts with AIza).');

const alertToken = typeof S.ALERT_TOKEN === 'string' ? S.ALERT_TOKEN.trim() : '';
if (alertToken && !/^[A-Za-z0-9]{24,}$/.test(alertToken)) fail('ALERT_TOKEN should be 24+ letters and digits (openssl rand -hex 16).');

const sa = S.GOOGLE_SERVICE_ACCOUNT;
if (sa && typeof sa === 'object' && Object.keys(sa).some((k) => k[0] !== '_')) { // "_note" alone = blank
  if (sa.type !== 'service_account' || !sa.client_email || !/BEGIN PRIVATE KEY/.test(sa.private_key || '')) {
    fail('GOOGLE_SERVICE_ACCOUNT must be the whole JSON key file of a service account (type, client_email, private_key).');
  }
}

const cals = S.CALENDARS;
if (cals != null) {
  if (!Array.isArray(cals)) fail('CALENDARS must be a list: [ { "url": …, "people": … } ]');
  let ids = [];
  try { ids = JSON.parse(readFileSync(new URL('settings.json', dir), 'utf8')).people.map((p) => p.id); } catch (e) {
    fail('can’t check CALENDARS without settings.json (its people ids).');
  }
  cals.forEach((c, i) => {
    const n = 'CALENDARS entry ' + (i + 1);
    if (!c || typeof c.url !== 'string' || !/^(https|webcal):\/\//.test(c.url.trim())) fail(n + ': "url" must be the https:// secret iCal address');
    const people = [].concat(c.people || []);
    const bad = people.filter((p) => p !== 'all' && ids.indexOf(p) < 0);
    if (!people.length || bad.length) fail(n + ': "people" must be "all" or ids from settings.json (' + ids.join(', ') + ')' + (bad.length ? '; unknown: ' + bad.join(', ') : ''));
  });
}

const out = {};
if (key) out.DASHBOARD_KEY = key;
if (token) out.HA_TOKEN = token;
if (cals && cals.length) out.CALENDARS = JSON.stringify(cals);
if (maps) out.GOOGLE_MAPS_KEY = maps;
if (alertToken) out.ALERT_TOKEN = alertToken;
if (sa && typeof sa === 'object' && sa.private_key) out.GOOGLE_SERVICE_ACCOUNT = JSON.stringify(sa);
const names = Object.keys(out);
const skipped = ['DASHBOARD_KEY', 'HA_TOKEN', 'CALENDARS', 'GOOGLE_MAPS_KEY', 'GOOGLE_SERVICE_ACCOUNT', 'ALERT_TOKEN'].filter((n) => !out[n]);
console.log('secrets.json OK: ' + (names.join(', ') || 'nothing filled in') + (skipped.length ? ' (blank, skipped: ' + skipped.join(', ') + ')' : '') +
  (cals && cals.length ? '; ' + cals.length + ' calendar(s), for ' + cals.map((c) => [].concat(c.people).join('+')).join(', ') : '') +
  (out.GOOGLE_SERVICE_ACCOUNT ? '; frame events written to Google as ' + sa.client_email + ' (share each calendar with it)' : ''));
if (mode === 'check' || !names.length) process.exit(0);

// ------------------------------------------------------------ local: .dev.vars for wrangler dev
if (mode === 'local') {
  const env = names.map((n) => n + "='" + out[n].replace(/'/g, "\\'") + "'").join('\n') + '\n';
  const dev = new URL('.dev.vars', dir);
  writeFileSync(dev, env, { mode: 0o600 });
  chmodSync(dev, 0o600);
  console.log('Wrote .dev.vars (' + names.join(', ') + ') for wrangler dev.');
  process.exit(0);
}

// ------------------------------------------------------------ upload: one request, values on stdin
const r = spawnSync('npx', ['wrangler', 'secret', 'bulk'], { cwd: dir, input: JSON.stringify(out), stdio: ['pipe', 'inherit', 'inherit'] });
process.exit(r.status == null ? 1 : r.status);
