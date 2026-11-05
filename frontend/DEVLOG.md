# Home HQ dashboard: development log

Technical notes for whoever works on this next: how it's built, why it's built that way, what was
measured, and what went wrong along the way. [README.md](README.md) covers setup and day-to-day use;
[kiosk/README.md](kiosk/README.md) covers the Android app. The version history is at the end,
newest first, with the commit each version shipped in.

## Contents

1. [Architecture](#architecture)
2. [The free plan's CPU budget](#the-free-plans-cpu-budget)
3. [Home Assistant](#home-assistant)
4. [Google](#google)
5. [Screens, night and sleep (per frame)](#screens-night-and-sleep-per-frame)
6. [Kiosk app](#kiosk-app-kiosk)
7. [Working on it](#working-on-it)
8. [Version history](#version-history)
9. [Open items](#open-items)

## Architecture

```
 Frames (kiosk WebView)                 Phones (/m, /upload)
   │ HTTPS + cookie key                    │
   │ WebSocket /api/alerts/ws              │
   ▼                                       ▼
 ┌──────────────────── Cloudflare Worker "home-hq" ─────────────────────┐
 │ static assets (Vite build, dist/) + /api/* (worker/index.js)         │
 │ D1 home-hq   groceries, events, notes, photos, cache (incl. the      │
 │              pushed Home Assistant states)                           │
 │ R2           home-hq-photos: full/<id>.jpg, small/<id>.jpg           │
 │ edge cache   (per data centre) HA snapshot, finished /api/home view, │
 │              last good camera pictures                               │
 │ AlertHub     Durable Object holding every frame's WebSocket          │
 │ cron */5     frame health -> Home Assistant notify                   │
 └──┬──────────────┬─────────────┬───────────────┬──────────────────────┘
    │              │             │               │
 Google iCal   Open-Meteo,   Home Assistant   Google Routes API,
 feeds (read)  Met Office    (Cloudflare      Google Calendar API
               RSS           Tunnel, REST)    (service account)
                                ▲
   HA automations POST /api/alert (urgent alerts) and /api/ha-push (states),
   both with ALERT_TOKEN
```

- **One Worker serves everything.** `wrangler.jsonc` binds `ASSETS` (the Vite output), `DB` (D1),
  `PHOTOS` (R2) and `ALERTS` (the `AlertHub` Durable Object), and runs `scheduled()` every 5
  minutes. It's on a custom domain; `workers_dev` is off. The tracked `wrangler.jsonc` and
  `tools/moved/wrangler.jsonc` carry placeholders; the real domain and D1 id live in a gitignored copy,
  `wrangler.local.jsonc`, which `tools/wr.mjs` (behind `npm run deploy`, `db:migrate` and `dev:api`)
  uses when it exists. A bare `wrangler deploy` would use the placeholders.
- **Config split.** `settings.json` (gitignored) is imported into the Worker at build time, so a
  settings change needs `npm run deploy`. `secrets.json` (gitignored, mode 600) is uploaded with
  `wrangler secret bulk` by `tools/secrets.mjs`, so a secret change needs no deploy. The browser
  only sees what the API chooses to send (names, colours, lists), never keys or tokens.
- **Auth.** `DASHBOARD_KEY` arrives once as `?key=`, moves into an HttpOnly cookie (`hq`, 10
  years) and is stripped from the address bar. Exceptions: `POST /api/alert` and `POST
  /api/ha-push` take `ALERT_TOKEN` instead (it can push alerts and states, nothing else), and `POST
  /share` (the Android share target) redirects to the upload page when it can't handle the request.
- **Front end.** Plain ES modules and IIFEs, no framework, built by Vite. `window.DATA` (`D`) is
  the shared state; `js/sync.js` fills it and emits `data`; views in `js/views/` register with
  `App.register` and redraw from `D`. The last good copy is kept in `localStorage`, so a frame
  starts instantly and survives outages. Saves are optimistic and roll back on failure.
- **Version.** `package.json` `version` becomes `__APP_VERSION__` (shown under Status in the display
  controls); the build writes `build.json` with a build id. Frames check it every 5 minutes and
  reload after a deploy once nobody has touched the screen for a minute (not while the screen is
  off, so a reload never lights it).
- **Must work on WebView 106** (the PF1007L, Android 6): ES2022 at most; check newer JS and CSS
  against [`docs/building-for-the-frame.md`](../docs/building-for-the-frame.md).
- **Sounds (1.9).** Synthesised with the Web Audio API (`js/sound.js`), no audio files. Chimes
  (doorbell, leave now, bins, new note) are quiet at night, in night mode and while everyone is
  away; smoke, CO and leak alarms always sound and repeat. Mute and volume are per frame.
- **Notes (1.14).** `notes` table; a note for one person is also pushed to their phone through
  `people[].notify`. Cleared notes keep their row so Undo works.
- **Bins (1.3).** Collection dates are computed from the council's rotation in `settings.json`
  (`bins.every`, `bins.moved`); the council has no usable lookup API. "They're out" is stored as
  `bins-out`. Bank-holiday moves go in `bins.moved`.
- **Share target (1.5).** `public/m.webmanifest` + `public/share-sw.js` put the phone page in
  Android's share menu; the images go to the upload page.

### Polling

| What | Day | Ambient / night | Notes |
| --- | --- | --- | --- |
| `/api/state` (shopping, notes, bins, frames) | 30 s | 30 s | |
| `/api/calendar` | 5 min | 5 min | the Worker re-parses the feeds at most every 10 min |
| `/api/weather` | 15 min | 15 min | includes Met Office warnings (10 min) |
| `/api/home` | 30 s | 60 s | from the pushed states; also refreshed at once on an alert or after a control |
| `/api/alerts/ws` | WebSocket | WebSocket | ping every 30 s; while it's down, `GET /api/alerts` every 10 s |
| camera pictures | 20 s on the Home tab | | 2 s for the doorbell takeover |
| `build.json` | 5 min | 5 min | auto-reload after a deploy |
| heartbeat `/api/frames` | 5 min | 5 min | kiosk 1.2+ or `?frame=` screens only |

### D1

| Table | Holds |
| --- | --- |
| `groceries` | shopping list; ticked items stay until cleared |
| `events` | events added on the frame; `gcal`/`gid` (0002) link the Google copy |
| `notes` | sticky notes (0003); cleared notes keep `cleared` so Undo works |
| `photos` | photo metadata; images in R2 |
| `cache` | key/value with expiry (below) |
| `chore_log` | unused since 1.3 (chores removed); kept so old data isn't lost |

`cache` keys: `calendar`, `weather`, `warnings`, `bins-out`, `frame:<id>` (heartbeats),
`frame-alerts`, `alert-seq` (the last alert, the fallback for a frame without a socket), `ha-push`
(the pushed Home Assistant states, with when they arrived), `ha-dirty` (set by a control or an alert:
the pushed copy is out of date until the next push), `energy-yday:<midnight>`, `energy-today`, the
drive-time lookups and the daily drive budget.

Migrations live in `migrations/` and are applied with `npm run db:migrate`. **Apply them remotely
and check before deploying code that needs them**: in 1.14 the notes migration hadn't applied and
`/api/state` failed for about a minute after the deploy. The Durable Object has its own migration
(`migrations` in `wrangler.jsonc`, `new_sqlite_classes`, the kind the free plan allows), applied by
the deploy.

## The free plan's CPU budget

Cloudflare's free plan allows **10 ms of CPU per request** (waiting for I/O is free) and 100,000
requests a day. Subrequests include D1 and R2 calls, with a limit of 50 per request.

**What happened.** The Cloudflare dashboard (Metrics, errors) showed **986 "Exceeded CPU Time
Limits"** in the week to 25 October: none before the 1.16 deploy (version `0c7b31ac`), the first
shortly after it, the most on 24 October when both frames were on, none after the 1.22 deploy (the
frames were off by then, so that isn't proven yet; check again after a day or two with them on).

**How it was measured.** `npx wrangler tail home-hq --format json` prints one event per request with
`cpuTime`, `wallTime`, `outcome` and the response status; the events are pretty-printed JSON, not
one per line. To split a request's cost into steps, bundle the Worker with esbuild
(`--bundle --format=esm --platform=node --loader:.json=json`), run `default.fetch` in Node with a
stand-in `caches.default`, `fetch` and D1, and use `node --cpu-prof`. Add
`crypto.subtle.timingSafeEqual` (Workers-only) to the stand-in.

**What it cost, in ms of CPU per request:**

| Request | 1.13-1.19 (entities one by one) | 1.20-1.21 (one read) | Now (1.22.1, pushed) |
| --- | --- | --- | --- |
| `/api/home`, refreshing | 14-43 | 14-32 | 1-6 typical; 10-22 once after an alert or a control (a direct read) |
| `/api/home`, cached | 2-10 | 1-3 | 1-3 |
| `/api/home/energy`, recomputing | 27-35 | 18 | 11 |
| `/api/calendar`, refreshing | 14 | | 9 |
| `/api/state` | 2-16 | | 3-5 |
| `/api/alerts` (the 1.16 long poll, 1.16-1.21) | **20-22** | **20-22** | gone |
| alert push (`POST /api/alert`) | | | 3-5 |
| state push (`POST /api/ha-push`) | | | 2-12 (median 5) |
| WebSocket connect / `AlertHub` | | | 1 / 0 |
| camera picture | 0-4 | | 0-4 |

**Where it went:**
- A refresh of `/api/home` is dominated by turning Home Assistant's whole state list (1,802
  entities, 834 KB, 110 KB gzipped) into objects to keep 21 KB (2.5%): about 2.6 ms to unpack, 6 ms
  (12 on a cold start) to `JSON.parse`, ~10% decoding the bytes to text, ~9% our filter loop.
  Reading the ~36 entities one at a time (1.13) cost the same in fetch overhead. The fix is to not
  receive the list: see [Pushed states](#home-assistant).
- The 1.16 alert long poll re-read D1 once a second for 25 s, about 0.85 ms each, so 20-22 ms per
  call. A push is a separate run of the Worker, which can't signal the waiting one, so polling D1
  was the only way to notice it. A Durable Object can be told directly: see [Alerts](#home-assistant).
- `caches.default` is **per data centre**, not global. Anything that one request writes for another
  to read must go through D1 (or a Durable Object), not the edge cache.
- A frame with Home Assistant now makes about 10,000 requests a day (Home and lists every 30 s,
  plus the pushes it triggers), well under the 100,000 limit.

## Home Assistant

- Reached through a Cloudflare Tunnel at the house, with a long-lived token of a non-admin
  "Frame" user. Non-admin users get **401 from `/api/template`**, so states are read with
  `/api/states` and `/api/history`.
- **How states reach the Worker.** Home Assistant pushes them, with a fallback to reading:
  1. **Push (1.21).** `npm run ha-state` prints a `rest_command` and an automation. The payload is
     templated inside Home Assistant (`expand([...])`, `states.media_player`, battery sensors,
     `to_json`) with a fixed list of 25 attributes (`KEEP` in `tools/ha-state.mjs`: the ones
     `worker/index.js` reads; add to it if the Worker starts reading another). It must not send
     `s.attributes` whole: `to_json` fails with "Type is not JSON serializable: datetime.datetime"
     on entities with a datetime attribute (media players have `media_position_updated_at`). The
     automation fires on a change of the quiet entities and every minute (which also carries the
     busy power and cost sensors); mode `queued`, 2 s between pushes. `pushState` (same
     `ALERT_TOKEN`) reduces it with `reduceStates` and stores `ha-push` in D1.
  2. **Read (fallback).** `haSnapshotFetch` uses the pushed copy while it is fresh (150 s) and newer
     than `ha-dirty`. Otherwise it does one `GET /api/states`, reduces it the same way and caches the
     result 8 s in the edge cache (`ha-snap`). A control or an alert sets `ha-dirty` (`forgetSnapshot(env)`),
     so the next reads go to Home Assistant itself and show the change at once, because the push for
     it hasn't arrived yet. `/api/home` is cached 8 s per location too (`home-view`).
  3. Every Home Assistant call has a timeout (15 s, history 30 s). After a failure the Worker stops
     asking for 30 s (`ha-fail`), so a struggling Home Assistant isn't hammered. Status in the
     display controls says "(pushed)" or "(read by the Worker)".
- **History of the read.** The first version fetched `/api/states` every 10 s per frame (800 KB x
  12 a minute) and the Home Assistant VM ran out of RAM. 1.13 read the ~36 entities one by one and
  did a full read for discovery every 5 minutes: kind to Home Assistant, but 14-27 ms of Worker CPU.
  The VM has since been fixed and copes with full reads, so 1.20 went back to one read, and 1.21 added
  the push so the Worker normally doesn't read at all.
- **Alerts (1.15, 1.16, 1.22).** Polling every 10 s plus an 8 s cache meant a doorbell could take
  ~20 s, so urgent entities are pushed: an automation (`npm run ha-alerts`) calls
  `rest_command.homehq_alert` -> `POST /api/alert?t=ALERT_TOKEN` with a type (`doorbell`,
  `smoke`, `leak`, `test`). The Worker bumps `alert-seq` in D1 and clears the snapshot. A
  `rest_command` only registers after a full Home Assistant restart, not a YAML reload.
  - 1.15 had the frames poll the counter every 2 s: 43,200 requests a day per frame, which put two
    frames over the 100,000 limit.
  - 1.16 held the request open in the Worker (a long poll): ~3,500 a day, but 20-22 ms of CPU per call.
  - **1.22: one WebSocket per frame to the `AlertHub` Durable Object** (`/api/alerts/ws`). `pushAlert`
    posts to the hub, which sends the alert to every open socket at once, so waiting costs nothing and
    an alert arrives in a fraction of a second. The hub hibernates between messages;
    `setWebSocketAutoResponse('ping','pong')` answers the frame's 30 s ping without waking it. A
    new socket is sent the last alert (the counter). The frame reconnects with a 2-30 s back-off and,
    while down, reads `GET /api/alerts` every 10 s so an alert is never missed. Pages from 1.16-1.21
    still send `?since=` and loop straight back on every answer, so the Worker holds that one call
    for 20 s with a single sleep until they have reloaded, instead of letting them spin through the
    request quota. Tested locally and live: two sockets both receive a push, ping/pong, a late joiner
    gets the latest, 401 without the key, reconnect after a server restart.
  - **Doorbell off switch.** `home.alerts.doorbell.off: true` drops the doorbell when settings load,
    so the alert, the camera takeover and the generated Home Assistant YAML all ignore it while its
    settings stay. Added while the new Blink doorbell wasn't ringing in Home Assistant.
- **Camera snapshots (1.22.1).** The Cloudflare log showed `/api/ha/camera?id=camera.<name>`
  failing every 20 s: status 500 after 10.03 s wall, 0 ms CPU, outcome "ok". That is Home
  Assistant's own 10 s `camera_proxy` timeout (the camera asleep or busy), passed through by the
  Worker, and the frame showed a broken-image icon. `cameraImage` now fetches with an 8 s timeout,
  keeps the last good picture per camera in the edge cache (6 h), and sends it with `x-stale:
  <seconds>` when Home Assistant errors, times out or answers with something that isn't an image.
  A camera with no kept picture gets a real 502/504. On the frame, `App.refreshCamera` fetches the
  picture as a blob and swaps it in only once it has arrived (the old code assigned `img.src`
  directly, and the doorbell takeover loaded each picture twice, because `no-store` made the second
  assignment a second request). The Home tab's age label shows the oldest picture's age. Why the
  driveway camera sometimes can't deliver is not solved (it is Home Assistant's or the camera's).
- **Media (1.11).** `playersView` finds every `media_player.*` in the states, merges duplicates by
  friendly name (Cast + Assistant SDK + old TV entries), ranks by `MEDIA_RANK`, and hides
  unavailable ones. `loaded` marks an idle player with nothing to resume; Music Assistant returns
  HTTP 500 for play on such a player, so the buttons are disabled and the Worker answers 409.
  Artwork: `/api/ha/art` uses `entity_picture_local` (Home Assistant's own proxy) and never fetches
  a LAN address: Music Assistant's `entity_picture` is `http://192.168.1.10:8095/...`, which
  Cloudflare can't reach (it was a 403).
- **Energy (1.10).** kWh today is integrated from `power` history (`wattHours`), hourly bars for
  today (cached 15 min) and yesterday (cached for the day). The DCC smart meter's `usage` sensors
  arrive hours late, so they're only used to say when the meter last reported.
  **History API gotcha:** without `end_time`, `/api/history/period/<start>` returns one day from
  `start`, not "until now". This once made a working smart meter look stuck for days.
- **Frame health (1.6).** Frames heartbeat every 5 min into `frame:<id>`. Same-name entries quiet
  15 min are deduped (a frame gets a new id at a new address); entries expire after 30 days. The
  cron sends one "offline" and one "back" notification per outage through `frames.notify`, with the
  same tag so Android replaces the first.

## Google

- **Drive times (1.7).** Routes API `computeRoutes`, `TRAFFIC_AWARE` (Pro SKU), origin from the
  latitude/longitude of the person's Home Assistant `person.` entity, or `zone.home`. Only events
  with a location starting in the next 4 h; each cached 10 min; over 3 h or no route falls back to
  `Travel:`. A Worker-side budget of 300 calls a day (`DRIVE_DAILY_MAX`) protects the bill, because
  free trial accounts can't set quotas in the console. `Walk:` minutes are added on top.
- **Calendar write-back (1.8).** No OAuth user login: a service account signs an RS256 JWT with
  WebCrypto (`googleToken`), trades it for an access token, and writes to calendars shared with it
  ("Make changes to events"). Calendar ids are parsed from the iCal URLs. The event row keeps
  `gcal`/`gid`; `getCalendar` hides the local copy once the feed contains `<gid>@google.com`, so
  nothing shows twice. A Google Workspace organisation can enforce
  `iam.disableServiceAccountKeyCreation`; the project needed a policy override to make the key,
  then was set back to inherit.

## Screens, night and sleep (per frame)

Everything here is kept **on the frame** (`localStorage`), set in the display controls, so each
frame can differ. Keys: `hq-night` (Dim, Bright, Sleep, Wake, Sleep on/off), `hq-idle` (timeouts),
`hq-muted` and `hq-volume`, `hq-frame-id` and `hq-frame-name`. The kiosk keeps its own
`screenoff` mode (below).

- **Night mode (1.17, 1.18).** Dim 22:00 to Bright 06:00 by default; Sleep 01:00 to Wake 05:00 off
  by default. A window may cross midnight; from = to means never. Times are `"HH:MM"` (`App.night`).
- **Timeouts (1.18).** Today 60 s, Rest 120 s, and Peek 20 s used instead of Rest during the night
  hours, so a look at the time in bed doesn't keep the screen up (`App.idle`; 5-3600 s).
- **The clock decides the resting screen** (`App.restMode`): screen off between Sleep and Wake if
  switched on, the dim clock between Dim and Bright, otherwise the photo screen. It is applied when
  the idle timer runs out (`App.rest`) and at the four boundaries for a frame resting untouched
  (`App.syncNightMode`, every minute). A frame being used (the day mode) isn't interrupted; it rests
  when it goes quiet. A tap wakes to Today whatever the screen was; the tap checks the clock for the
  tint.
- **The tint.** While it's night the whole dashboard is dimmed and amber (`data-night` on the stage,
  a CSS filter on the rail and views), whatever the mode, tapped awake or not. Alerts aren't under
  `.views`, so they're never dimmed. This replaced 1.17's 20 s "nightdash" mode.
- **Modes dropdown.** Ambient = `App.rest()` (rest now, the clock still decides, and keeps switching
  at the boundaries). Screen off = a manual pick (`App.setMode('off', true)`): it stays, whatever the
  time, until a tap. The keyboard's **N** (night preview) was removed; **A** does the same as Ambient.
- **Screen off, the page side.** A tap on the off layer is ignored for 1.5 s (a screen takes a second
  to go dark; a bounce on the button that turned it off woke the PF1007L 1.2 s later). A page that
  loads while the screen is asleep joins "off" (`info().screenOn`). `window.HomeHQScreenOn()` (called
  by the kiosk when the screen comes on) wakes to Today.

## Kiosk app (`kiosk/`)

- Built without Gradle (`build.sh`/`build.ps1`: javac, d8, aapt2, apksigner). 1.2 = versionCode 3;
  1.3 = 4, 1.3.1 = 5, 1.3.2 = 6, 1.3.3 = 7, 1.4 = 8, 1.4.1 = 9.
- **Bridge.** `window.HomeHQKiosk` (1.2): `setScreenOff(bool)` and `info()` (kiosk version, model,
  Android version, Wi-Fi RSSI; 1.3 adds `sleep`, `screenoff` and `screenOn`). The kiosk calls
  `window.HomeHQScreenOn()` in the page when the screen comes on.
- **Screen off has three modes**, chosen once per frame with
  `am start -n com.wesbos.homehq/.MainActivity --es screenoff <lock|backlight|dim|auto>`:
  - **`lock` (1.3, the C-2).** The kiosk is a device admin with only `force-lock` (`ScreenAdmin`; enable
    with `dpm set-active-admin com.wesbos.homehq/.ScreenAdmin`). `setScreenOff(true)` calls `lockNow()`,
    the same as the power button, and holds a partial wake lock so the CPU and Wi-Fi keep running.
    Measured on the C-2: `backlight(0)` in `/sys/class/disp/disp/attr/sys`, the page still checked in 3
    minutes later while dark, and one tap woke it straight to Today. Window brightness 0 doesn't do
    this: it only reaches the panel minimum (8/255 on the C-2, 10/255 on the PF1007L).
  - **`backlight` (1.3.1-1.3.3, the PF1007L).** On the PF1007L the touch controller (gslX680) powers
    down with the display and sent no events at all while asleep (recorded with `getevent`), so a
    tap can't wake it. Instead the frame stays awake and the kiosk writes 0 to the Allwinner display
    driver's `/sys/class/disp/disp/attr/lcdbl` (root-only; world-readable) through `su`. Three things
    learned: this frame's Android 6 `su` takes `su 0 sh -c "..."` and rejects `-c` ("invalid uid/gid
    '-c'"; 1.3.2 tries both forms); Android's auto-brightness pushed `lcdbl` back up whenever the
    room light changed (0 -> 242, the panel lit for ~9 s with a 5 s re-apply loop), so while dark
    the kiosk sets `screen_brightness_mode` to manual (`settings put system`), waits 0.4 s, writes
    0, re-applies every 0.5 s as a backstop, and on wake writes the saved level then restores auto
    (1.3.3); and a write that didn't take falls back to dimming rather than staying lit (1.3.1 did,
    which is how the `su` problem showed). `autobright_off` records that auto-brightness is owed back,
    so a kiosk killed while dark puts it right on the next start.
  - `dim`: window brightness 0. `auto`: forget the choice (`lock` if the admin right is on, else `dim`).
- **Wireless ADB (1.4).** A 5-minute window started from the display controls (`Bridge.setAdb`), the
  same as `adb tcpip 5555` through `su` (`setprop service.adb.tcp.port 5555; stop adbd; start adbd`),
  closed again with port `0`. Both frames run root ADB with `ro.adb.secure=0` (no key check), so it is
  off by default, only started at the frame, and limited to 5 minutes. **From 1.4.1 it refuses to start
  until a subnet is saved** (`--es adbfrom 192.168.1.0/24`, validated as an IPv4 CIDR because it goes into
  a root command; `any` opens it to the network on purpose, `none` removes the value); 1.4 opened to the
  whole network without one. With the subnet saved the kiosk first creates an `iptables` chain `homehq_adb` (accept the subnet, drop the rest, jumped to from
  INPUT for tcp/5555) and an `ip6tables` DROP, and **fails closed**: if the rules can't be made, ADB
  stays closed and `adbError` says so. The deadline is kept in prefs (`adbuntil`) and the timer is static,
  one shared `Handler` and task (an instance field let a recreated activity leave an old callback that
  closed a newer window early); `recoverAdb` on start keeps a running window going after an
  install restarts the kiosk, and closes one that has run out. `adbd` is only restarted on close if the
  network port was on, so a USB session isn't dropped for nothing. The page shows the state, a
  countdown and `adb connect <ip>:5555` (the kiosk reports `adbIp` from `WifiManager`), and hides the
  section on kiosks without `adbLeft`. Tried by the owner on 2026-10-26 on both frames: 1.4 installed over USB,
  then the same build installed over Wi-Fi with `deploy-wifi.sh` (both frames, on a different subnet from the
  computer); both worked. On the PF1007L the lock
  was read back afterwards (saved, window closed, no firewall rules left).
- **Deploy scripts (private, `frames/`).** `deploy-usb.sh` (the old `deploy.sh`, with a guard that
  refuses a network address) and `deploy-wifi.sh <address>`, a wrapper: it tests TCP 5555 first (a
  refusal means the frame is reachable but ADB is off; a time-out means the network is blocking it),
  `adb connect`s, exports `ADB_SERIAL` and `DEPLOY_WIFI`, runs the USB script's steps, and
  disconnects on exit. The change log records which one ran.
- **Saved URL:** the kiosk only saves a URL given by `--es url` (`scripts/set-kiosk-url.sh`).
  Redirects are followed but not saved, so after the move to `homehq` each frame needed re-pointing
  before the old Worker could be deleted.
- **Updating a frame:** `frames/<frame>/build.sh` and `deploy.sh` (private, gitignored). Deploy checks
  the fingerprint and signing certificate, `adb install -r`, **`sync`**, restarts, checks
  `mResumedActivity` (Android 10 doesn't print `mCurrentFocus`), and offers to enable the sleep right.
  On 2026-10-19 the C-2 lost power shortly after an install: the package list was saved but the APK
  wasn't, so it booted to a black screen with no Home app. Hence the `sync` and the "leave it
  powered 30 s" message.
- adb in scripts must read `/dev/null`, or it swallows the next line of stdin (the y/N answer).
- **PF1007L:** `adbd` runs as root; plain `cat` works for prefs. It has no volume buttons; media
  volume is set with `service call audio 3 i32 3 i32 <0-15> i32 0 s16 com.android.shell`. USB drops
  and returns once about 45 s after boot, and the power must come through the powered hub.
- **C-2:** re-pointing needs a USB-C cable that carries data and fits its recessed port. Never
  `pm disable` the whole `com.waophoto.fota` package (every activity crashes at boot).

## Working on it

- **Code review, 2026-10-28** (the changes since 1.15; ten findings, nine fixed in 1.23.1 / kiosk 1.4.1,
  each checked against the code first, and each fix tested, the Worker ones against the old code too):
  - **Alert socket:** a connection with no pong for 75 s was only `close()`d, and the close handshake can
    take very long on a half-open connection, keeping the fallback poll off while doorbell, smoke and leak
    pushes were missed. It is now dropped and reconnected at once.
  - **`ha-dirty` was stamped after the control returned,** so the push that control triggers (which can
    arrive first) was judged stale and forced full 800 KB reads for up to a minute. It is now stamped before
    (`forgetSnapshot`), and only the edge copies are dropped after (`dropSnapshots`). Test: a push arriving
    during the control is served from the push; the old code did a full read.
  - **Wireless ADB refuses to start without a saved subnet** (see Kiosk app).
  - **The ADB section re-rendered every second,** replacing the button under a finger, so some taps were lost;
    now only the countdown text changes while the state is the same.
  - **`refreshCamera`:** one refresh at a time per picture (a request can take 8 s while the doorbell asks every
    2 s, so an older answer could land on a newer one), the new picture is preloaded and only swapped in if it
    loads, blob URLs are released (on error, when replaced, and when the takeover closes).
  - **The camera "now" label** said "now" while a refresh was still failing; it now counts up until a round
    succeeds. The fix first broke the 20 s refresh (resetting the counter whenever the view re-rendered, which it
    does often); the browser test caught that.
  - **Backlight mode:** a brief pause (`onPause` lights the panel) left the panel lit while the page still
    showed "off"; `darkWanted` re-darkens it on resume.
  - **Artwork:** an absolute address is fetched only if it is `https`, within 8 s, and only an `image/*` answer
    is passed on, with `nosniff` and a sandboxing CSP (also on camera pictures); the old LAN deny-list missed
    cases and wrongly blocked hosts starting `10.`.
  - **Not fixed:** `expand()` in the `ha-state` template replaces a group or light group with its members, so
    a configured group entity would never be pushed. It doesn't affect these entities (their pushed payload
    returned each as itself) and can't be tested without Home Assistant; revisit if a group is ever added.

- **`wrangler deploy` ships the working folder as it is, not the last commit.** On 2026-10-25 an
  unexplained, uncommitted `refreshBlink` (re-trigger a Blink camera at most every 10 s on each look
  at the doorbell camera) went out with three deploys before it was noticed. Run `git diff` before
  deploying, and don't assume who made an uncommitted change. It was removed, with its README
  paragraph and the `refresh` setting.
- Local preview: `.claude/launch.json` "dashboard-local" (`wrangler dev`, port 8787, test key `dev`
  from `.dev.vars`; `.claude` is gitignored). The local Worker has no Home Assistant.
- `secrets.json` values may be used in commands for checks (curl against the live Worker, Home
  Assistant, Google) but are never printed: read into a shell variable, mask, unset.
- `pkill -f` or `ps | grep` patterns that also appear in the command line of the shell running them
  kill that shell (exit 144). Write the pattern so it can't match itself, or kill by PID.
- Scratch copies of Home Assistant data (the state list holds positions and camera tokens) belong
  in a scratch folder and are deleted afterwards.
- **Upstream merge and frame checks, 2026-11-02** (merged `wesbos/main`, 8b19d47; both frames on kiosk
  1.4.3). From the upstream issues, applied by hand and logged in each frame's private `changes.jsonl`:
  the PF1007L boot scripts re-applied (`install.sh apply`) for the Wi-Fi restart when Android gives up
  starting the driver (#221; check the local `frame-boot.conf` MAC matches the frame's before applying, it
  is gitignored and had been blanked); the AAA Certificate Services root (`75680d2e.0`) copied to
  `/data/misc/user/0/cacerts-removed/` on both frames for the compromised eMudhra chains (#229; two
  advisory hosts refused in the kiosk on each; undo by deleting the file). #256 (remount overlay on
  `/cache`) does not apply to the C-2: its overlay is on `scratch`, nothing listens on 5555 at boot and
  14035 is dropped. The IPv6 block for wireless ADB works on the C-2 (`ip6tables` present).
  Not adopted: #259 (clock re-poll), #278 (VIEW link scheme), #253 (frame firewall); see the PRs.

## Version history

Dates are 2026; the commit is the one the version shipped in (a commit can hold several versions).

| Version | Commit | Date | What changed |
| --- | --- | --- | --- |
| 1.23.1 / kiosk 1.4.1 | 6ff060c | 28 Oct | Code review fixes: wireless ADB refuses without a subnet; the alert socket reconnects at once when it stops answering; `ha-dirty` stamped before a control; camera refreshes one at a time and preloaded; ADB section stops losing taps; backlight re-darkens on resume; ADB timer static; artwork https-only and image-only |
| 1.23.2 / kiosk 1.4.2 | 997f838 | 29 Oct | Second code review, nine fixes. Kiosk: wireless ADB also needs the IPv6 block to succeed; subnets must be /16 or narrower (`any` is the only way to open it wide); an off-timer that runs outside the app (detached shell with a token) so a crash or update can't leave it open; start and stop serialised so a stop can't be undone by a start still running. Frame: a hung camera fetch times out after 12 s; blob URLs freed when a takeover is replaced or the cameras are redrawn; the camera age label only resets when every picture refreshed. Worker: artwork cached 10 min by the browser (its address changes per track); a refused or failed control no longer leaves `ha-dirty` set |
| kiosk 1.4.3 | ca2f061 | 29 Oct | Backlight mode, PF1007L: no flash going dark (the manual brightness setting is lowered while auto-brightness is still in charge, and given back after it's on again; Android otherwise jumps the panel to that setting for ~0.4 s when auto is switched off) and a dimmer start on wake when the old level is unknown (60, not 255). Checked on the PF1007L by eye: both flashes gone or much reduced |
| 1.23 / kiosk 1.4 | 96017a3 | 26 Oct | Wireless ADB: a 5-minute window from the display controls, limited to a saved subnet with firewall rules that fail closed; two deploy scripts per frame (USB and Wi-Fi) |
| 1.22.1 | 75f706e | 25 Oct | Camera snapshots keep the last good picture and are swapped in only once loaded (Home Assistant's 10 s camera timeout caused a 500 every 20 s); media artwork via `entity_picture_local`; doorbell `off` switch; unexplained `refreshBlink` removed |
| 1.22 | 9319129 | 25 Oct | Alerts over a WebSocket to the `AlertHub` Durable Object: instant, no polling, ~0 CPU idle (the 1.16 long poll cost 20-22 ms per call); old pages' `?since=` loop held for 20 s |
| 1.21 | 9319129 | 25 Oct | Home Assistant pushes the dashboard's states to `POST /api/ha-push` (`npm run ha-state`); the Worker reads them from D1 and falls back to its own read when they are stale or a control happened (`ha-dirty`); Status shows "(pushed)" |
| 1.20 | 9319129 | 25 Oct | Worker CPU: one `/api/states` read again (reduced, cached 8 s), the finished `/api/home` cached 8 s, home poll 30 s in the day, discovery removed |
| kiosk 1.3.3 | d2e8152 | 24 Oct | `backlight` mode turns auto-brightness off while dark, holds at 0.5 s, recovers if killed |
| kiosk 1.3.2 | d2e8152 | 24 Oct | `backlight` mode's `su` call: the PF1007L's `su` takes `su 0 sh -c`, not `-c` |
| 1.19.1 / kiosk 1.3.1 | d2e8152 | 24 Oct | Per-frame `screenoff` mode (`lock`, `backlight`, `dim`, `auto`) because the PF1007L can't wake from a real sleep; page ignores a tap on the off layer in the first 1.5 s |
| 1.19 / kiosk 1.3 | 2d9ec88 | 24 Oct | Real screen sleep: device admin with `force-lock`, `lockNow()` plus a wake lock, `HomeHQScreenOn()`, a page loading while asleep joins "off", no auto-reload while off |
| 1.18 | 5665152 | 24 Oct | Per-frame night and screen-off times and timeouts; the tint follows the clock; Modes is a dropdown (Ambient = rest now, Screen off stays until a tap); refresh is an icon; no `nightdash`; "Idle: fast" and the Night preview removed |
| 1.17.1 | 5665152 | 24 Oct | Night and screen-off time boxes open the picker anywhere in the box |
| 1.17 | 5665152 | 24 Oct | Night and screen-off hours per frame in the display controls; frames switch between the photo screen and the night clock by themselves |
| 1.16 | 875b4ef | 22 Oct | Pushed alerts by long poll (fits the free plan); this DEVLOG; README audit; dead `home.media` setting removed |
| 1.15 | 1b413cb | 21 Oct | Home Assistant pushes urgent alerts (`/api/alert`, a 2 s counter poll, `npm run ha-alerts`) |
| 1.14 | 596a69a | 21 Oct | Sticky notes on frames and phone; push to the person's phone; migration 0003 |
| 1.13 | 41dad00 | 21 Oct | Home Assistant load cut by >95%: per-entity reads, 8 s edge snapshot, 5 min discovery, timeouts, back-off |
| 1.12.1 / 1.12 | 0a54964, 0aa0e7a | 21 Oct | Home HQ wordmark top right (centred); presence sits on the rule |
| 1.11 | 4b6b8f3 | 21 Oct | Media tab for every player; kitchen card removed |
| 1.10 | 8092d38 | 21 Oct | Energy "today so far" panel; heating & water and climate summary removed; thermostat "Idle" |
| 1.9.1 / 1.9 | dbae8b3, ca6a4f9 | 21 Oct | Web Audio sounds with per-frame mute and volume; bins wording fix |
| 1.8.3 / 1.8.1 | 162bf84, fcf09c7 | 20 Oct | Add-event date and time pickers; 24 h calendar grid opening at 6 AM |
| 1.8 | 476d07d | 20 Oct | Frame events written to Google Calendar (service account); migration 0002 |
| 1.7 | 50e99d7 | 20 Oct | Live Google drive times for leave-by, `Walk:`, 300/day cap |
| (rename) | 3589960 | 20 Oct | Worker renamed `home-hq` on its custom domain; `tools/moved` redirect (the old Worker is deleted) |
| (config) | d616ca2 | 20 Oct | `settings.json` + `secrets.json` replace `family.json` + `calendars.json` |
| 1.6 | 3002512 | 20 Oct | Frame-offline phone notifications; tidier frame list |
| 1.5 | f25f2a5 | 19 Oct | Android share target for photos |
| 1.4 / kiosk 1.2 | 2d41c0c | 19 Oct | Screen off when away, all clear, frame health, auto-reload after a deploy |
| 1.3.2 / 1.3.1 / 1.3 | 7249b83, 51a79e0, 5c40b5c | 19 Oct | Chores removed; bins, leave countdown, Met Office warnings, energy line; test alerts dropdown |
| 1.2.1 / 1.2 | 5b19c69, effd15c | 19 Oct | Version under Status; time preview removed; "HA connected" |
| pre-1.2 | 7e4f916 ... b94881a | 16-19 Oct | Real data sources, phone page, smoke/CO takeover, per-calendar errors, `Travel:` parsing |

## Open items

- **Check the CPU counter** (Cloudflare, Workers & Pages, home-hq, Metrics) after a day or two with
  both frames on; "Exceeded CPU Time Limits" should stay at zero.
- Three "Uncaught Exception" events in the same chart have not been identified (Cloudflare's Logs,
  filtered to errors, will name the path and message).
- The driveway camera sometimes can't produce a snapshot within Home Assistant's 10 s (check Home
  Assistant's log for the driveway camera's entity, and what kind of camera it is).
- The doorbell is **off** (`"off": true`): the new Blink doorbell doesn't ring in Home Assistant. Turn
  it back on by deleting the word once there's a sensor that does; then re-run `npm run ha-alerts`.
- Kiosk 1.4.1 (the review fixes) is built but not on the frames: install it over Wi-Fi with `deploy-wifi.sh`,
  which is also a test of the new rule that a subnet must be saved.
- Wireless ADB (kiosk 1.4) works on both frames, with the subnet lock (`adbfrom`) saved on both: read back
  over USB on the PF1007L; on the C-2 it was saved by the owner afterwards (the first Wi-Fi install there ran
  with the 5-minute window open to the whole guest Wi-Fi) and has not been read back.
- Not yet watched on a real frame: the Sleep/Wake schedule, and an alert waking a dark frame.
- Kiosk 1.3.3 isn't on the C-2 yet (it's on 1.3, `lock`, which works); the PF1007L is on 1.3.3
  (`backlight`) and still has the unused device-admin right enabled (Android 6 has no
  `dpm remove-active-admin`; deactivate under Settings, Security, Device administrators if wanted).
- Second person's `notify` service and Home Assistant app location (for drive times and notes to their
  phone).
- Energy `rate` and `standing` aren't set, so there's no cost estimate.
- Name the frames with `&frame=` once they have permanent homes.
- Phones: reinstall the web app from `homehq` so the share menu entry points at the new address.
- Add Christmas bin dates to `bins.moved` when the council publishes the 2026/27 calendar.
- Idea, not started: self-hosting the Worker (Node + SQLite + a folder for photos) would remove the
  free plan's limits and let it reach Home Assistant on the LAN; about a day of adapter work.
