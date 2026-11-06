# Home HQ

A self-hosted backend for the Home HQ family dashboard. It runs on a wall-mounted photo frame,
with a phone page at the same address. The frontend is
[sambartle/photo-frame-dashboard](https://github.com/sambartle/photo-frame-dashboard)'s
`dashboard/`, vendored unmodified under [`frontend/`](frontend/). This service replaces
upstream's Cloudflare Worker with a small Rust (axum) binary that speaks the same `/api`
contract and embeds the Vite build. The full plan is in
[docs/prd-002-home-hq-kiosk.md](docs/prd-002-home-hq-kiosk.md).

## Status: delivery 1, the look with dummy data

The service serves the unmodified frontend (wall view at `/`, phone page at `/m`) and answers
every route the frontend calls. The responses use upstream's shapes, filled with dummy data from
[`src/demo.rs`](src/demo.rs).

- The clock, calendar views, weather, bins, notes, shopping, photos, Home controls, media and
  energy all render. Events and bins are generated relative to "now" in the configured
  timezone.
- Writes (shopping, notes, "They're out", controls, frame check-ins) change an in-memory copy,
  so the UI feels alive. Nothing persists across restarts.
- Upstream's key → cookie flow is implemented. `?key=` on a page sets the `hq` cookie, and the
  API accepts that cookie or `x-hq-key`, compared in constant time. Without `HQ_KEY` the
  dashboard is open.
- The alert WebSocket (`/api/alerts/ws`) and the polling fallback (`/api/alerts`) answer with
  `seq: 0`, and nothing raises alerts yet.

**Calendar (delivered).** With `HQ_CALENDARS` set, "Next up", the people lanes and the calendar
views show real ICS feeds through the `CalendarSource` port ([`src/calendar/`](src/calendar/)),
with recurrences, exceptions, timezones and per-person attribution. Reading iCalendar is its own
crate, [`crates/ical-expand`](crates/ical-expand/), whose README explains why it isn't a library. Without it, the demo events
stay.

Each later delivery replaces one slice of `demo.rs` with a real port and adapter:

| Delivery | Replaces |
|---|---|
| Store | SQLite (embedded migrations) for notes, bin state, frames and shopping |
| Bins | the real rotation from `settings.bins`, moved dates, the 4 PM window |
| Calendar | the ICS `CalendarSource` with recurrence, exceptions, timezones and attribution |
| Weather | Open-Meteo for `settings.location`, cached |
| HA gateway | state snapshot, actions, camera/art proxy, WebSocket alerts, presence |
| Frames monitor | offline notification through HA after 15 minutes |
| Capabilities | `/api/state` capability flags, plus the frontend patch that hides unconfigured tabs |
| Integrations | Immich, Mealie, CalDAV, travel time |
| Auth | Keycloak login for phones, frame pairing, step-up for sensitive controls ([RFC](docs/rfc-device-auth.md)) |

## Running locally

```bash
cd frontend && npm ci && npx vite build && cd ..
cargo run
```

Open <http://localhost:8080/> for the wall view and <http://localhost:8080/m> for the phone
page. `cargo test` works without building the frontend first, because `build.rs` creates an
empty `frontend/dist`.

| Variable | Default | Meaning |
|---|---|---|
| `HQ_LISTEN` | `0.0.0.0:8080` | Listen address (plain HTTP, always on) |
| `HQ_TLS_CERT` / `HQ_TLS_KEY` | unset (no TLS) | PEM certificate chain and key. Both set adds a TLS listener; the files are re-read every minute and a renewed certificate is used without a restart |
| `HQ_TLS_LISTEN` | `0.0.0.0:8443` | TLS listen address |
| `HQ_KEY` | unset (open) | Dashboard key (secret) |
| `HQ_CALENDARS` | unset (demo events) | Calendar feeds (secret): a JSON list of `{ "url": "…", "people": ["alex"] \| "all" }`. See [Calendars](charts/home-hq/README.md#calendars) |
| `HQ_SETTINGS` | bundled [`config/settings.example.json`](config/settings.example.json) | Household settings in upstream's `settings.json` format |
| `HQ_IMMICH_PUBLIC_URL` | unset (Share hidden) | Immich address phones open, e.g. `https://photos.example.com`. The Photos tab's Share button shows a QR code for `<url>/photos/<assetId>` (a plain link, no key) on photos from Immich. Must be `http(s)://host[/path]`, else startup fails |
| `HQ_WEATHER` | `open-meteo` | `open-meteo` serves Open-Meteo's forecast for `settings.location`, cached 15 minutes; a failed refresh serves the last forecast. With no `location`, `/api/weather` answers `{ "none": true }`. Needs egress to `api.open-meteo.com`. `demo` serves the generated demo forecast and makes no outbound calls (CI, offline development) |
| `HQ_OPEN_METEO_URL` | `https://api.open-meteo.com/v1/forecast` | Forecast endpoint, e.g. a self-hosted Open-Meteo |
| `RUST_LOG` | `info` | Log filter (logs are JSON) |

`GET /healthz` returns `ok` without a key.

### Photo frame behaviour

After eight seconds without interaction in Photos, the daytime frame shows only an
edge-to-edge slideshow: navigation, captions, notes, clock, weather and controls are hidden.
The first touch restores the Photos controls without triggering a photo action. Other views
use the existing configurable rest timeout before entering the same image-only ambient mode.
Night and screen-off schedules still take precedence, and open dialogs keep the normal timeout.
Photo colours stay unchanged at night; only the dashboard UI receives the night tint.

Use the circular expand icon at the top left in Photos to keep the current slideshow on screen
until you exit. Only the subtle circular shrink icon remains visible at the top left; Escape or navigating away also exits. This mode
suspends the app's idle, night/scheduled-off and everyone-away sleep transitions, and postpones
automatic deploy reloads. Normal timers resume from the time you exit. The browser's native
full-screen API and screen wake lock are used where available; unsupported or denied browser
APIs still allow the UI-only full-screen view. The Android kiosk already keeps the screen awake.
This mode is local to the current page and is not restored after reload.

### Connect Immich by QR code

Home HQ supports [`PedDavid/immich-device-auth`](https://github.com/PedDavid/immich-device-auth)
as an alternative to configuring a static Immich API key. Deploy the broker alongside Immich,
enable Immich OIDC sign-in, and register the broker's OAuth callback with your identity provider
as described in that repository. Then configure Home HQ:

```sh
HQ_IMMICH_URL=https://photos.example.com/api
HQ_IMMICH_DEVICE_AUTH_URL=https://device.photos.example.com
```

Leave `HQ_IMMICH_API_KEY` unset in device-auth mode. Both URLs are server configuration;
the browser cannot supply a broker or change where Home HQ sends a key. The configured
Immich URL must address the same server as the broker, and includes `/api` (the broker URL
does not). Internal service URLs are supported; the broker must return an approval URL reachable
from your phone. Use HTTPS in deployment and set `HQ_KEY` to restrict dashboard and pairing access.
With no dashboard key, pairing management is open just like the rest of the dashboard.

Open **Photos → Connect Immich** (or `/immich`), scan the QR code, compare the displayed code,
and approve on your phone. Home HQ requests `asset.read asset.view asset.update`, with a
one-day lifetime capped by the broker. The device code and issued key stay on the backend;
the QR image is generated locally. Polling respects the broker interval and `slow_down` responses.
Once connected, use **Show favorite photos**. **Disconnect** asks the broker to revoke the
issued key (`POST /device/revoke`); the key itself is the only credential needed. The broker
accepts the request and deletes the key in Immich itself, retrying in the background if Immich
is unavailable, so a successful Disconnect means the broker has the request, not that the key is
already gone. Only if the broker can't be reached, or answers with an error, does Home HQ keep
the grant so you can retry.

A grant is shared by this Home HQ instance and all its frames. By default it is held in memory
only: restarting Home HQ requires pairing again, and the broker cleans up the old key at expiry.
To keep it across restarts, set `HQ_STATE_DIR` to a writable directory (the Helm chart's
`persistence` option, on by default with a dashboard key, does this). Home HQ then stores the issued key, its expiry and scope in
`$HQ_STATE_DIR/immich-grant.bin`, encrypted with AES-256-GCM under a key derived from `HQ_KEY` and
bound to the two configured URLs, and reloads it on start. Without `HQ_KEY` nothing is written.
A stored grant that has expired or can't be decrypted (a changed `HQ_KEY` or URL) is deleted on
start, and the frame shows "disconnected". **Disconnect** deletes the file once the broker has accepted the revocation.
Expired grants stop being used even if the broker is unavailable. There is no refresh token;
return to `/immich` to approve again after expiry. Cancelling a pending request clears it locally;
the broker expires abandoned approvals and revokes any uncollected keys. Without `HQ_STATE_DIR`,
read-only container filesystems need no writable volume. Helm users can pass the two URLs through `extraEnv`.

The flow was tested end to end with the broker at commit
`1af7484a9a71b3946f14b7f569c3c5d440f98563` and its mock identity provider/Immich server
(extended locally with fixture photo endpoints). Real Immich/OIDC deployment validation is still
required; the broker's README lists its current proof-of-concept limitations.

### Immich favorites

Set both `HQ_IMMICH_URL` (the Immich API base URL, e.g. `https://photos.example.com/api`)
and `HQ_IMMICH_API_KEY` to replace demo photos with the API key owner's favorite images.
Keep the key in a secret/environment binding; never put it in household settings or browser code.
Use HTTPS outside local development. The key needs Immich asset search, view and update
permissions (`asset.read`, `asset.view`, `asset.update` on versions with scoped keys).
In Helm, pass these variables through `extraEnv`, using `secretKeyRef` for the key.

The adapter follows metadata search pagination (up to 10,000 images), caches the list for
60 seconds, and provides thumbnail/preview images through the authenticated dashboard API.
Video assets are excluded. Capture date, city, orientation and favorite status feed the existing
slideshow; favorites changed in the dashboard update Immich and invalidate the list cache.
Unfavoriting removes that photo from the next list. Other Immich changes appear after the cache
expires. Only listed favorite assets can be fetched or changed through this integration.
An empty library shows no photos; upstream errors return a generic 502 rather than demo images.
Images have a 20 MiB limit and upstream requests have a 15-second timeout. Redirects are disabled.
The API URL must include `/api`; incomplete configuration prevents startup. With neither variable
set, demo mode is unchanged. Albums and memories are not yet implemented.

Run `cargo test --locked --test immich` for the mock-server integration checks. These do not
require an Immich account; deployment should also be checked against your Immich version.
The adapter uses the metadata-search page API supported by Immich v2; current upstream
also exposes it but marks these search filters/pagination deprecated as of v3.2.

### Check the frame layout without deploying

With the built Rust server running, run from `frontend/`:

```bash
npx playwright install chromium  # once, unless using an installed Chromium
npm run test:layout
npm run test:photo-colours
npm run test:photo-fullscreen
```

For an installed browser, use `CHROMIUM_PATH=/usr/bin/chromium npm run test:layout`.
`LAYOUT_BASE_URL` overrides the default server address (`http://127.0.0.1:8080`);
`LAYOUT_OUTPUT` overrides the screenshot directory (`/tmp/home-hq-layout`).
The check covers the PF1007L's 1280×800 viewport, smaller viewports and a wider
screen. It blocks external requests, verifies bundled fonts load, and checks that
the clock and cards fit. Screenshots use a fixed four-digit PM clock. This is a
desktop Chromium layout check, not an Android WebView 106 emulator; device font
rasterization and Android system bars can differ. The frame's density should be
160, as described in `frontend/kiosk/README.md`.

### Weather snapshot

`tests/fixtures/weather/open-meteo-snapshot.json` is a real Open-Meteo response for the bundled
example settings, together with the request that produced it. `cargo test` maps it offline. To
fetch a new one, run `HQ_RECORD_WEATHER=1 cargo test --test weather_snapshot` (needs network), or
run the CI workflow by hand with *record-weather-snapshot* ticked. CI records the snapshot by
itself when the file is missing and commits it to the branch. The test fails when the request
changes (fields, units, the example location) until the snapshot is recorded again.

The `weather-open-meteo` screenshot test renders the card and hourly strip from this snapshot
(`frontend/tests/screens/fixtures/weather-open-meteo.json`, written by `cargo run --example
screen_fixtures`). Recording a new snapshot regenerates that fixture too, and CI commits both.
Then run CI by hand with *update screenshots* ticked, which renders and commits the new
`weather-open-meteo.png` baseline, and review it.

### Screenshot tests

`cd frontend && npm run test:screens:docker` renders the wall view at the frame's 1280×800 with
mocked API data, compares it with `frontend/tests/screens/screenshots/`, and checks that nothing
overflows the frame. `npm run test:screens:docker:update` rewrites the baselines. CI's `screens`
job runs the same in the same image and uploads the results as the `screenshots` artifact. See
[`frontend/tests/screens/README.md`](frontend/tests/screens/README.md), which also covers how to
put screenshots in a pull request.

## Container image

The [`Dockerfile`](Dockerfile) builds the frontend with Node, builds the binary with the
frontend embedded, and ships it on `distroless/cc` as non-root. Woodpecker
([`.woodpecker.yml`](.woodpecker.yml)) runs fmt, clippy and tests on every push, and builds and
pushes the image on tags. It uses the `registry_*` secrets. While the repo lives on GitHub,
GitHub Actions runs the same checks (`ci.yml`). On `v*` tags it also builds and pushes
`ghcr.io/peddavid/home-kiosk` for amd64, plus the Helm chart (`release.yml`).

## Deployment

- [`charts/home-hq/`](charts/home-hq/): the Helm chart, published to
  `oci://ghcr.io/peddavid/charts/home-hq` on each `v*` tag, together with the image of the same
  version. Its README has the `HelmRelease` for the Flux repo and the bootstrap steps.
- [`docs/frame-runbook.md`](docs/frame-runbook.md): the one-time PF1007L setup (router rules,
  ISRG roots in the trust store, kiosk app, checks).

## Updating the vendored frontend

`frontend/` is a squashed `git subtree` of upstream's `dashboard/` folder. To update it:

```bash
git clone https://github.com/sambartle/photo-frame-dashboard /tmp/upstream
git -C /tmp/upstream subtree split --prefix=dashboard -b dashboard-split
git fetch /tmp/upstream dashboard-split
git subtree pull --prefix=frontend --squash FETCH_HEAD -m "Update vendored frontend"
```

Then check the API usage (`grep -rn "App.api\|App.write\|/api/" frontend/js`) against
[`src/api.rs`](src/api.rs). Local changes to the frontend are listed in [PATCHES.md](PATCHES.md).

## Licence

Upstream is MIT (© Wes Bos and the fork's contributors). Its licence is kept in
[`frontend/LICENSE`](frontend/LICENSE).
