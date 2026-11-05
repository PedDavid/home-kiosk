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
| `HQ_LISTEN` | `0.0.0.0:8080` | Listen address |
| `HQ_KEY` | unset (open) | Dashboard key (secret) |
| `HQ_SETTINGS` | bundled [`config/settings.example.json`](config/settings.example.json) | Household settings in upstream's `settings.json` format |
| `RUST_LOG` | `info` | Log filter (logs are JSON) |

`GET /healthz` returns `ok` without a key.

### Photo frame behaviour

After eight seconds without interaction in Photos, the daytime frame shows only an
edge-to-edge slideshow: navigation, captions, notes, clock, weather and controls are hidden.
The first touch restores the Photos controls without triggering a photo action. Other views
use the existing configurable rest timeout before entering the same image-only ambient mode.
Night and screen-off schedules still take precedence, and open dialogs keep the normal timeout.

### Immich favourites

Set both `HQ_IMMICH_URL` (the Immich API base URL, e.g. `https://photos.example.com/api`)
and `HQ_IMMICH_API_KEY` to replace demo photos with the API key owner's favourite images.
Keep the key in a secret/environment binding; never put it in household settings or browser code.
Use HTTPS outside local development. The key needs Immich asset search, view and update
permissions (`asset.read`, `asset.view`, `asset.update` on versions with scoped keys).
In Helm, pass these variables through `extraEnv`, using `secretKeyRef` for the key.

The adapter follows metadata search pagination (up to 10,000 images), caches the list for
60 seconds, and provides thumbnail/preview images through the authenticated dashboard API.
Video assets are excluded. Capture date, city, orientation and favourite status feed the existing
slideshow; favourites changed in the dashboard update Immich and invalidate the list cache.
Unfavouriting removes that photo from the next list. Other Immich changes appear after the cache
expires. Only listed favourite assets can be fetched or changed through this integration.
An empty library shows no photos; upstream errors return a generic 502 rather than demo images.
Images have a 20 MiB limit and upstream requests have a 15-second timeout. Redirects are disabled.
The API URL must include `/api`; incomplete configuration prevents startup. With neither variable
set, demo mode is unchanged. Albums and memories are not yet implemented.

Run `cargo test --locked --test immich` for the mock-server integration checks. These do not
require an Immich account; deployment should also be checked against your Immich version.
The adapter uses the metadata-search page API supported by Immich v2; current upstream
also exposes it but marks these search filters/pagination deprecated as of v3.2.

## Container image

The [`Dockerfile`](Dockerfile) builds the frontend with Node, builds the binary with the
frontend embedded, and ships it on `distroless/cc` as non-root. Woodpecker
([`.woodpecker.yml`](.woodpecker.yml)) runs fmt, clippy and tests on every push, and builds and
pushes the image on tags. It uses the `registry_*` secrets. While the repo lives on GitHub,
GitHub Actions runs the same checks (`ci.yml`). On `v*` tags it also builds and pushes
`ghcr.io/peddavid/home-kiosk` for amd64 and arm64, plus the Helm chart (`release.yml`).

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
