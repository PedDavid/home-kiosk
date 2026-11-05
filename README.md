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

## Container image

The [`Dockerfile`](Dockerfile) builds the frontend with Node, builds the binary with the
frontend embedded, and ships it on `distroless/cc` as non-root. Woodpecker
([`.woodpecker.yml`](.woodpecker.yml)) runs fmt, clippy and tests on every push, and builds and
pushes the image on tags. It uses the `registry_*` secrets. While the repo lives on GitHub,
GitHub Actions runs the same checks (`ci.yml`). On `v*` tags it also builds and pushes
`ghcr.io/peddavid/home-kiosk` for amd64 and arm64 (`release.yml`).

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
