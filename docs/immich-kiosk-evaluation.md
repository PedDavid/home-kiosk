# Evaluation: Immich Kiosk as Home HQ's photo viewer

**Status:** Research, not validated on the device
**Issue:** [#10](https://github.com/PedDavid/home-kiosk/issues/10). Related: [#7](https://github.com/PedDavid/home-kiosk/issues/7) (Home → Photos continuity), [#8](https://github.com/PedDavid/home-kiosk/issues/8) (metadata readability), PR [#1](https://github.com/PedDavid/home-kiosk/pull/1) (Immich favourites and `immich-device-auth`), PR [#9](https://github.com/PedDavid/home-kiosk/pull/9) (kiosk APK)
**Date:** 2026-11-06

## Summary

[Immich Kiosk](https://github.com/damongolding/immich-kiosk) (AGPL-3.0, Go + htmx) is a mature,
very configurable Immich slideshow. It officially supports Chrome ≥ 106, which is exactly the
frame's WebView. It keeps its Immich API key on the server and proxies every image, so the key
never reaches the browser. The problems are how it fits into Home HQ:

1. **It can't open a given photo.** No URL parameter or API makes the slideshow start on an
   asset ID, so #7 (tap the Home photo, see that photo in Photos) can't be done without an upstream
   change or a fork.
2. **It needs a static, non-empty API key.** It exits (`log.Fatal`) when the key is empty,
   including during a config reload. It has no notion of expiry or revocation. Backend-only keys
   with expiry and revocation from `immich-device-auth` are possible, but only if Home HQ sits
   between Kiosk and Immich and injects the real key (see [Keys](#4-immich-device-auth-keys-backend-only-expiry-revocation)).
   That is new product code in Home HQ.
3. **The frame can't reach it.** The frame may only talk to `192.168.8.56:443`. Kiosk has no
   base-path support: it uses absolute paths, including `/assets/`, which clash with Home HQ's
   own. So Home HQ would have to serve it under a second hostname and proxy it, and the
   certificate needs a new SAN.
4. **Home HQ loses sight of touches.** A cross-origin iframe hides its pointer events from the
   parent, and Kiosk has no `postMessage` API. Home HQ's idle, exit and alert logic has to work
   around the iframe instead of with it.

**Recommendation:** keep the custom viewer as the default and fix #7 and #8 there. They are
small changes in code we own. Treat Immich Kiosk as an optional, flagged "Photos (Immich Kiosk)"
view, and build it only if the owner wants its extra features (memories, people, albums, split
views, transitions, offline mode) badly enough to pay for the proxy, the hostname and #7. Before
writing any integration code, do a short spike on the frame (Phase 0 of the
[migration plan](#9-migration-plan)) using the [on-device checklist](#10-on-device-test-checklist).

Everything here comes from reading Immich Kiosk's source and docs. The source is
`damongolding/immich-kiosk` at `3aff816` (v0.47.0, 2026-11-05). The docs are
`damongolding/immich-kiosk-docs` at `41d586e`, because docs.immichkiosk.app wasn't reachable
from the research environment. Nothing was run on the PF1007L. **[device]** marks claims that
need a check on the frame.

## Context: what Home HQ has today

- **Frame:** PF1007L, Android 6, WebView 106, 1280×800. It runs the kiosk APK: the one in
  `frontend/kiosk/` (on main), or wesbos's in `android/` (PR #9, which the frame is said to run).
  Both use immersive full screen and `FLAG_KEEP_SCREEN_ON`. Only the `frontend/kiosk/` APK
  injects `window.HomeHQKiosk` (`setScreenOff`, `setAdb`, `info`). Neither sets a
  `WebChromeClient`, so the HTML Fullscreen API has no native implementation.
- **Network** ([frame runbook](frame-runbook.md) §1): the frame's only allowed destination is
  the dashboard's own LAN address, `192.168.8.56:443`, where the Rust app terminates TLS. It has
  no internet access.
- **Photos** (PR #1, not merged; the "latest custom layout prototype" the issue calls shelved):
  - `src/immich.rs` turns Immich favourites into upstream's photo contract, proxies thumbnails
    and previews through `/api/photos/:id`, and only serves assets that are listed favourites.
  - `src/device_auth.rs` runs the RFC 8628 flow against `immich-device-auth`. The grant (scope
    `asset.read asset.view asset.update`, 1-day TTL, hard cap of 7 days) lives only in memory. It
    is dropped at expiry even if the broker is down. Disconnect revokes it through the broker.
    The browser only ever sees `status`, `user_code`, the verification URI and a QR code that the
    server generates.
  - The frontend (`photos.js`, `slideshow.js`) has the Auto, Cover, Pair, Collage and Polaroid
    layouts, a blur fill for portraits, captions, favourites, the 8-second quiet frame and an
    explicit full screen that holds off sleep.
- **Navigation, idle and alerts** live in the parent page (`app.js`, `sync.js`): a document-level
  `pointerdown` resets the idle timer, the clock picks ambient, night or off, and Home Assistant
  alerts come in over the WebSocket and open toasts or sheets.

## 1. Embedding: iframe or separate route

| | iframe inside Home HQ (recommended) | Separate top-level route |
|---|---|---|
| Home HQ nav rail, alerts, schedules | Keep running in the parent | Gone while Photos is open. A doorbell alert can't show. |
| Way back | Parent-owned controls | Only Kiosk's `redirects` menu, the kiosk's native menu or a reload |
| Touch zones | Kiosk's zones inside the iframe, parent controls outside it | Kiosk's zones only |
| Complexity | Hostname, proxy, idle workaround | Same hostname and proxy. The UX regression stops it. |

A separate route loses alerts and the exit controls, so it fails the issue's second scope item.
**Use an iframe.**

**Constraints on the iframe:**

- **No sub-path.** Kiosk builds every URL from `/`: htmx `hx-post="/asset/new"`, `/image`,
  `/sse`, `/assets/...` and a catch-all `/:redirect`. There is no base-path option. Its
  `/assets/` would also clash with Vite's `/assets/` in Home HQ. So Kiosk needs **its own
  origin**, for example `photos.home-hq.prdv.cloud`.
- **The frame's network.** That hostname must resolve to `192.168.8.56`, and the Home HQ TLS
  listener must route it by `Host` to Kiosk's ClusterIP Service. That needs a SAN or wildcard
  on the Actalis certificate and a Pi-hole record. No firewall change is needed. The alternative,
  a second MetalLB IP plus a router rule plus TLS in front of Kiosk (which has none), widens the
  frame's reach for no gain.
- **Framing headers.** Kiosk sends no `X-Frame-Options` or CSP `frame-ancestors`, so framing
  works today. Home HQ's proxy should add `frame-ancestors https://home-hq.prdv.cloud`.
- **Suggested element:**
  `<iframe src="https://photos.home-hq.prdv.cloud/" sandbox="allow-scripts allow-same-origin allow-forms" referrerpolicy="no-referrer">`.
  - Leaving out `allow-popups` and `allow-top-navigation` stops Kiosk's `target="_blank"` links
    (the "more info" asset link, Google Maps) from replacing the dashboard. In a WebView without
    multi-window support, a `_blank` link otherwise loads in the main view. **[device]**
  - `allow-same-origin` is safe here because the frame's origin differs from the parent's. Kiosk
    needs it for storage, its service worker and cookies.

**Compatibility risks on WebView 106:**

| Risk | Finding | Severity |
|---|---|---|
| Minimum browser | Docs and `browserslist`: `chrome >= 106`. CSS is built with `--target=chrome106`, JS with `--target=es6`. The only modern selector found is `:has()` (Chrome 105+), in the slide transition. No `dvh`, `@container`, `color-mix`, `@starting-style` or `popover`. | Low today. **High over time:** 106 is the floor, so any later bump breaks the frame. Pin the image version and check the changelog on every upgrade. |
| Upstream fallback | `immich-kiosk-legacy` exists for older browsers. It's an "experiment" and not needed at 106. | Info |
| JS APIs | esbuild lowers syntax only, not APIs. `AbortSignal.timeout` (103+) is fine. | Low **[device]** |
| Fullscreen | `fullscreen.ts` calls `requestFullscreen` on `<body>`. In an iframe this needs `allow="fullscreen"`, and in a WebView it needs `WebChromeClient.onShowCustomView`, which neither APK has. Expect a no-op. The kiosk is already immersive. | Low. Just don't rely on it. **[device]** |
| Wake lock | `disable_screensaver` uses `navigator.wakeLock`. Android WebView is expected not to support it. Not needed: the APK sets `FLAG_KEEP_SCREEN_ON`. | None |
| Service worker | Kiosk registers `/assets/js/sw.js` for offline mode. The WebView supports it. Caching inside an iframe needs checking. | Low **[device]** |
| SSE | `/sse` is a long-lived stream through Home HQ's proxy. The proxy must not buffer it or time it out. | Medium (proxy design) |
| Cookies in the iframe | `photos.home-hq.prdv.cloud` inside `home-hq.prdv.cloud` is same-site. Chromium should treat its cookies as first-party, but WebView's `setAcceptThirdPartyCookies` defaults to off. It's unclear whether the WebView applies it per site or per origin. | Medium **[device]**. It decides how the proxy authenticates the frame (§4). |
| JS bridge exposure | `addJavascriptInterface` exposes `window.HomeHQKiosk` to **every frame**, iframes included. With the `frontend/kiosk/` APK, any script in Kiosk's origin could call `setAdb(true)` on a root-ADB device. wesbos's `android/` APK has no bridge. | **High** if `frontend/kiosk/` is used. Use the `android/` APK, or move the bridge to `WebViewCompat.addWebMessageListener` with an origin allowlist. |
| Touch | Kiosk uses click zones (left = previous, top-centre = pause and menu, right = next). It has no swipe gestures. The gslX680 panel merges two-finger touches, but Kiosk needs none. | Low **[device]** |

## 2. Keeping Home HQ's navigation, exit controls and alerts

The parent page stays the shell, and Kiosk fills the view area:

- **Navigation:** in normal Photos mode the nav rail stays visible and the iframe fills
  `.view`. Home HQ owns every way out. Kiosk's own menu (top-centre tap) stays for
  pause, previous and next. Hide anything that leaves Kiosk: set `more_info.display: false` or
  turn off `asset_link` and `qr_code`, no `redirects`, and `show_clear_cache_button: false`.
- **Full screen and quiet frame:** reuse the explicit full screen from PR #1. The parent hides
  the rail and draws only its **Exit full screen** button above the iframe (higher `z-index`), in
  a corner that doesn't overlap Kiosk's zones. A tap on that button lands on the parent.
- **Idle:** the parent can't see touches inside the iframe, so `pointerdown` → `resetIdle`
  never fires while someone uses Kiosk. Treat the Kiosk view as an image view:
  - Suspend the "go home after N seconds" and "rest" transitions while it is current, as
    `App.photoFullscreen` does now.
  - Keep the clock-driven night and off schedules, which don't depend on touches.
  - Without a `postMessage` hook upstream, there is no reliable way to detect "the user is
    interacting". Proxying in a shim script that relays clicks to the parent would mean
    modifying Kiosk's output (see §7), so leave it out.
- **Alerts:** `sync.js` keeps running in the parent, and toasts and sheets render above the
  iframe. For alerts that navigate, such as a doorbell opening the camera, `App.go()` hides the
  view. Then unload the iframe or set `src="about:blank"` so Kiosk stops polling. **[device]**:
  check that a toast over the iframe renders and takes taps.
- **The native escape** (two-finger hold, or PR #9's pull-down) is handled in Java
  `dispatchTouchEvent` before the WebView, so it works over the iframe too.

## 3. Home → Photos continuity, fitting and metadata

### Continuity (#7)

Not supported:

- Kiosk's `/` route takes no "start at this asset" parameter. The first asset comes from
  `POST /asset/new`, chosen by the server from the configured buckets with weighting.
- The `history` form field only drives previous and next within a session.
- `GET /image/:imageID` can return any single image, and v0.46 added an
  `X-Immich-Asset-ID` header to `/image`, but neither starts the slideshow on a given photo.

Options:

1. Ask upstream for an `asset=<id>` (or `start_asset`) URL parameter, or contribute one. It
   would push the given asset to the front of history on the first load. This is the right fix
   and keeps us on the official image.
2. Run a patched fork. The AGPL then requires offering the source of the patched version to
   its network users (§7), and we'd carry a rebase on every release.
3. Accept that the Home tile and Photos are separate slideshows. This is the bug #7 describes.

The Home tile could keep using Home HQ's own `/api/photos` adapter, but #7 stays open until
option 1 or 2 lands.

### Portrait and landscape on 1280×800

| Need | Kiosk option |
|---|---|
| Portrait on a landscape screen without cropping | `layout: single`, `image_fit: contain`, `background_blur: true`. Same idea as Home HQ's blur fill. |
| Two portraits side by side | `layout: splitview`, like Home HQ's "Pair" |
| Fill the screen | `image_fit: cover`, `image_effect: smart-zoom` (needs `face.read` scope) |
| Collage or polaroid | None. Home HQ's Collage and Polaroid would be lost. |

Kiosk works out orientation from EXIF. Its docs warn that an image without EXIF can be laid out
in the wrong orientation. **[device]** Check one landscape, one portrait and one no-EXIF image in
each layout.

### Metadata readability (#8)

- Options: `show_image_date`, `image_date_format`, `show_image_location` (with
  `hide_countries` and per-part hiding), `show_image_time` and `font_size` (percentage).
- `theme: solid` draws a solid band behind the metadata. That reads better on bright or busy
  photos than `fade`, a gradient.
- `custom.css`, mounted at `/custom.css`, is applied last, which allows a stronger scrim, a
  bigger type size or a different position. Target this frame with `custom_css_class` if needed.
- `disable_ui: true` gives a clean idle frame, but these settings apply all the time, not only
  when details are shown. Kiosk has no "show details on tap, hide when idle" mode, which #8
  asks for. The nearest is the "more info" overlay (key `i`, or the menu), which also carries
  links we'd hide.
- **Missing EXIF and rendering problems:** Kiosk leaves out fields that are absent, so a photo
  with no date shows nothing and can't be told apart from a hidden date. **[device]** Compare an
  asset with full EXIF and one without, and confirm in the Immich UI.

## 4. `immich-device-auth` keys: backend only, expiry, revocation

What Kiosk supports:

- **Sources for the key:** `immich_api_key` in `config.yaml`, `KIOSK_IMMICH_API_KEY`,
  `KIOSK_IMMICH_API_KEY_FILE` (a Docker or Kubernetes secret file) or a systemd credential.
  `immich_users_api_keys` maps extra users, and only works from `config.yaml`.
- **Server-side only.** The key goes out as the `x-api-key` header from the Go server. Images
  and video are proxied through `/image*`, `/video/:id` and `/live/:id`. The redacted `/config`
  endpoint and the about page never print it. The browser never gets the key.
- **No expiry or revocation.** The key is read once at start-up. It's read again only when
  `watch_config: true` sees `config.yaml` change: its mtime or hash. A change to the key file
  alone doesn't trigger a reload. A reload reloads every connected client.
- **An empty key is fatal.** `checkRequiredFields()` calls `log.Fatal("Immich API key is
  missing")`, at start-up and during a reload. "Disconnect by blanking the key" would crash the
  container.
- **The key isn't scoped to the slideshow.** `GET /image/:imageID` fetches any asset ID with the
  configured key, not just assets in the configured albums or favourites. Anyone who can reach
  Kiosk can read every asset the key can view.
- **`immich_url` keeps only scheme and host.** Endpoints are built as `{scheme}://{host}/api/...`,
  so a path on `immich_url` is dropped. Whatever Kiosk calls has to be at the root of its own
  host and port.
- **Requirements:** Immich v3.2.0 or newer (docs). Its full-feature permission list is long:
  asset, album, albumAsset, archive, face, memory, person, server.about, tag and user. A
  favourites-only, view-only setup needs far fewer.

**Design that meets the issue's rules: Home HQ as an Immich gateway.**

```
frame ──TLS──▶ Home HQ :8443 ─Host: photos.home-hq…─▶ Kiosk :3000 ──▶ Home HQ gateway :8081 ──x-api-key: <device grant>──▶ Immich
         (HQ key check, Kiosk password header,          (holds only a        (checks gateway token,
          path allowlist, frame-ancestors)               gateway token)       path and asset allowlist)
```

1. Kiosk's "Immich API key" is a random **gateway token** from a Kubernetes Secret, mounted
   with `KIOSK_IMMICH_API_KEY_FILE`. It is never empty, so Kiosk never exits, but it's useless
   against Immich itself.
2. `immich_url` points at a new **internal-only** Home HQ listener, for example
   `http://home-hq-gateway:8081`. A ClusterIP Service and a NetworkPolicy admit only the Kiosk
   pod.
3. The gateway checks the token in constant time, removes it, and attaches the current grant
   from `DeviceAuth::source()`. With no grant, or an expired one, it returns **401** without
   calling Immich.
4. **Expiry and revocation take effect on the next request.** Kiosk needs no restart, there's no
   key file to rewrite and no reload. The grant stays in Home HQ's memory, as in PR #1.
5. **Allowlist:**
   - **Paths:** `api/server/{ping,version}`, `api/search/random` and `api/search/metadata` with
     the favourites filter forced in the body, `api/assets/{id}` and
     `api/assets/{id}/thumbnail`.
   - **Asset IDs:** only IDs that `Immich::contains()` already accepts, so the `/image/:id`
     overreach is closed on the Immich side.
   - **Writes:** a favourite `PUT` only if the like button is kept (`like_button_action: favorite`).
   - **Everything else** gets 403: albums, tags, archive, faces, people, users and downloads.
6. **Kiosk's cache:** `kiosk.cache` and `prefetch` keep recent API responses and images in
   memory. After a disconnect, Home HQ should call Kiosk's `/cache/flush` over the internal
   Service. That endpoint is blocked on the public hostname.
7. **Broker scope:** keep `asset.read asset.view asset.update`. **[device]**/staging: run Kiosk
   with `debug_verbose` against the gateway, list the Immich calls it makes for the chosen
   config, and widen the allowlist or scope only if something essential is missing
   (`api/users/me` for `show_owner`, for example, which we wouldn't use).

**Public side.** The Home HQ proxy in front of `photos.home-hq.prdv.cloud`:

- Requires the HQ key. That means giving the `hq` cookie `Domain=home-hq.prdv.cloud`; today
  it's host-only (`src/auth.rs`). If cookies in the iframe turn out to be blocked **[device]**,
  use a short-lived signed token in the iframe URL, swapped for a cookie by the proxy.
- Adds Kiosk's own `password` as an `X-Api-Key` header, so the browser never holds it. Kiosk's
  docs call this password weak protection, so it's a second layer, not the boundary.
- Blocks `/config`, `/about` (which also calls api.github.com), `/url-builder*`,
  `/cache/flush`, `/webhooks`, `/redirects/*` and query overrides. Also set
  `disable_url_queries: true` and `disable_config_endpoint: true`.

**Rejected alternatives:**

- A shared tmpfs key file plus `watch_config`: the key sits on disk, disconnect crashes Kiosk
  (empty key), and a reload needs a `config.yaml` touch.
- A static key in a Secret: no expiry, and revocation means rotating by hand.
- Giving the key to the browser: not possible with Kiosk, and against the requirement anyway.

**Conclusion:** Kiosk can work without the key ever reaching the browser, but **not on its
own**. Backend-only keys with expiry and revocation need the gateway. That is roughly the
current `immich.rs` plus a generic proxy, and it's the largest piece of new work in this
option.

## 5. Sleep behaviour

- **Kiosk's sleep:** `sleep_start` and `sleep_end` (24-hour times, also settable by URL)
  switch the client to a black screen with an optional faint clock. `sleep_dim_screen` only does
  anything in Fully Kiosk Browser or the ImmichFrame app. `disable_sleep=true` bypasses it.
- **Home HQ should own sleep.** It already has the idle → ambient → night/off modes, and the
  `frontend/kiosk/` APK's `setScreenOff`. Leave Kiosk's sleep unset so the two schedules can't
  disagree. When Home HQ enters `off` (or leaves Photos), unload the iframe to stop the SSE
  stream and image fetches, then reload it on wake. Waking makes Kiosk fetch a new asset
  anyway.
- **Wake lock** is irrelevant (§1). The APK keeps the screen on, and Home HQ decides when it's
  dark.
- **[device]:**
  - Kiosk in the iframe through night → off → touch-to-wake. Check no black flash, that it
    resumes within a few seconds, and that check-ins continue.
  - An hour on screen: watch memory (`dumpsys meminfo com.wesbos.homehq`) for growth from the
    SSE stream and image swaps on the A33.

## 6. Deployment next to the Helm chart

Add it to the same chart, off by default. Don't make it a sidecar: separate pods keep the Home
HQ Deployment's Recreate strategy and resource limits simple.

- **Values:** `immichKiosk.enabled`. Pin `image` to an exact version **and digest**; Renovate
  may propose updates, but each one needs the Chrome 106 check. Also `hostname`, `config` (a
  `config.yaml` map) and `customCss`.
- **Kiosk Deployment:**
  - One replica, `distroless/static:nonroot` (upstream image), `runAsUser: 65532`,
    `readOnlyRootFilesystem`, all capabilities dropped. Requests about 20m CPU and 64Mi memory,
    limit about 256Mi **[staging]**.
  - `config.yaml` from a ConfigMap at `/config` (WORKDIR is `/`). Kiosk refuses a `./config`
    directory whose mode isn't `0755`. Check what the ConfigMap volume mount produces
    **[staging]**.
  - Secrets: the gateway token at `KIOSK_IMMICH_API_KEY_FILE` and the password at
    `KIOSK_PASSWORD_FILE`. Liveness and readiness on `/health`.
  - `TZ` set to the household timezone. Upstream's image defaults to `Europe/London`.
- **Services:** a ClusterIP Service for Kiosk, and a second ClusterIP Service for Home HQ's
  gateway port.
- **NetworkPolicies:**
  - Kiosk ingress only from Home HQ.
  - Kiosk egress only to the gateway, plus DNS.
  - The gateway port admits only Kiosk.
  - Kiosk needs no internet: fonts are bundled, and weather and iframes stay off.
- **Home HQ side:**
  - New env vars: `HQ_PHOTO_KIOSK_UPSTREAM`, `HQ_PHOTO_KIOSK_HOST` and `HQ_IMMICH_GATEWAY_LISTEN`.
  - The TLS certificate gains the SAN.
  - The Ingress can take the extra host for LAN phones.
- **Infra:** a Pi-hole record for the new hostname pointing at `192.168.8.56`. The router rule
  is unchanged.
- **Suggested Kiosk `config.yaml`:**

  ```yaml
  album: [favorites]
  layout: splitview
  image_fit: contain
  background_blur: true
  theme: solid
  show_image_date: true
  show_image_location: true
  font_size: 120
  transition: cross-fade
  duration: 15
  disable_ui: false
  more_info: { display: false }
  show_videos: false
  kiosk:
    behind_proxy: true
    disable_url_queries: true
    disable_config_endpoint: true
    enable_url_builder: false
    watch_config: false
  ```

## 7. AGPL-3.0

This is not legal advice, but the setup is a common one.

- **Unmodified upstream image as a separate service:** no source obligation. §13 (remote
  network interaction) applies only to a *modified* version. We don't convey (distribute) the
  program either: we pull upstream's public image. Keep its licence and notices intact.
- **Home HQ stays MIT.** An iframe and an HTTP proxy are arm's-length communication between
  separate programs, not a combined work ("aggregate"). Home HQ's code, and upstream's vendored
  MIT frontend, are unaffected.
- **Things that would trigger obligations:**
  - **Patching Kiosk** (for example a start-asset parameter for #7, §3). Every user who
    interacts with it over the network must be offered the Corresponding Source of the running
    version. In practice: a public fork tagged to match the deployed image, a link to it in the
    Kiosk UI (the about page, or the footer through `custom.css` content), and AGPL-3.0 kept on
    the fork.
  - **Publishing a modified image** (for example on GHCR) is conveying. Provide the source, the
    licence and the notices.
  - **Rewriting Kiosk's responses in the proxy** (injecting scripts or changing its HTML/JS) is
    arguably a modification. Avoid it, and keep the proxy to headers and path filtering.
  - **Copying Kiosk code into Home HQ** (its TS, CSS or Go) would make that part of Home HQ
    AGPL. Don't port code. Re-implementing ideas is fine.
- **`custom.css` and `config.yaml`** are inputs the program reads, not changes to it. They can
  stay in this repo under MIT. Publishing them is harmless and simplifies any future source offer.
- **Upstream route:** contributing the start-asset feature upstream avoids a fork and all of the
  above.

## 8. Tradeoffs against the custom viewer

| | Custom viewer (PR #1) | Immich Kiosk |
|---|---|---|
| #7 Home → Photos continuity | Small change: pass the ID to the Photos slideshow | Blocked on upstream or a fork |
| #8 Metadata readability | Fully ours: tap for details, hidden when idle | Always on or off; `custom.css` for contrast |
| Navigation, idle, alerts | Native: one document | Workarounds: no touch events from the iframe |
| Keys: backend only, expiry, revocation | Done (`device_auth.rs`), grant in memory | Needs a new gateway listener and allowlist |
| Network and TLS | Nothing new | New hostname, SAN, DNS, Host routing, SSE proxying |
| Sources | Favourites only (albums and memories not built) | Albums, people, tags, dates, memories, ratings, timelines, multi-user |
| Presentation | Five layouts, Ken Burns, blur fill | Single, split views, transitions, smart zoom, burn-in protection |
| Video and live photos | No | Yes (heavy for the A33) |
| Offline | No | Offline mode with a service worker |
| Browser risk | We test against WebView 106 | 106 is upstream's floor. Every upgrade is a risk. |
| Upkeep | Our code | Upstream ships often (v0.46 and v0.47 two days apart). Pin and review. |
| Licence | MIT | AGPL, simple if unmodified |
| Immich version | v2 metadata search (deprecated in v3.2) | Needs ≥ v3.2 |

**Alternative considered:** use Kiosk only as an image source (its `/image` endpoint with the
`X-Immich-Asset-ID` header) behind Home HQ's own viewer. That keeps all the plumbing, gives up
Kiosk's UI, and only replaces selection logic we already have. Not worth it.

## 9. Migration plan

**Phase 0: spike, no product code, about an evening on the frame**

1. Run Kiosk with a **temporary, view-only, short-expiry** Immich key on a LAN host.
2. Open it directly in the frame's WebView. Temporarily add the URL as the kiosk URL, or as a
   second router rule to that host for the spike only, then remove it.
3. Then open it inside a static test page served by Home HQ in an iframe.
4. Work through the [checklist](#10-on-device-test-checklist) and take the screenshots.
5. **Stop** if the WebView rendering, touch or memory fails, or if the owner decides the
   features don't justify the plumbing. Then close #10 and finish #7 and #8 in the custom viewer.

**Phase 1: upstream ask (in parallel with Phase 0)**

- Open an issue or PR on `damongolding/immich-kiosk` for a start-asset URL parameter.
  Optionally also ask for a `postMessage` event for user interaction, which would fix the idle
  problem in §2.

**Phase 2: backend (only if Phase 0 passes)**

1. Merge or rework PR #1's `immich.rs` and `device_auth.rs`.
2. Add the gateway listener with the token check, the path and asset allowlist, and 401 when
   disconnected. Test it against the mock broker and a mock Immich, as `tests/immich.rs` does.
3. Add Host-based reverse proxying for `HQ_PHOTO_KIOSK_HOST`:
   - SSE pass-through, `frame-ancestors`, the path blocklist and the password header.
   - The HQ-key check with the cookie `Domain` change.

**Phase 3: chart and infra**

1. Add the `immichKiosk` values, Deployment, Services and NetworkPolicies (§6). Run `helm lint`
   with `ci/test-values.yaml` extended.
2. Reissue the certificate with the SAN and add the Pi-hole record.

**Phase 4: frontend behind a flag**

1. Add a `photoViewer: "custom" | "immich-kiosk"` setting. With `immich-kiosk`, the Photos view
   renders the sandboxed iframe, suspends idle transitions, keeps the Exit control, and
   unloads the iframe on `hide()` and in `off` mode.
2. Record the patch in `PATCHES.md`.
3. Keep the custom viewer as the fallback, and keep the Home tile on `/api/photos`.

**Phase 5: cut over**

1. Run on the frame for a week with the checklist re-run.
2. Make Kiosk the default only if #7 is solved (upstream or a fork) or accepted as a known gap.
3. Keep the custom viewer for one release before removing it.

**Rollback:** set `photoViewer: custom` and `immichKiosk.enabled: false`. Nothing else depends
on Kiosk.

## 10. On-device test checklist

Run on the PF1007L with the installed kiosk APK. Note which APK: `android/` or `frontend/kiosk/`.
Attach screenshots (`adb -d exec-out screencap -p > shot.png`) to #10.

**Rendering and compatibility**

- [ ] Kiosk loads directly in the WebView 106 kiosk. No console errors (`--ez debug true`,
      then `chrome://inspect`).
- [ ] The same inside the sandboxed iframe on a Home HQ test page.
- [ ] `cross-fade` transition is smooth, with no tearing or long blank frames. Note any jank.
      Try `transition: none` for comparison.
- [ ] `background_blur` at 1280×800 doesn't stall the A33. If it does, check whether
      `optimize_images: true` helps.
- [ ] Fonts render (bundled woff2) with no network requests outside `192.168.8.56`.
- [ ] Kiosk's offline icon appears when Home HQ is stopped, and it recovers by itself.

**Touch, navigation and exit**

- [ ] Left and right taps change the photo, and a top-centre tap opens Kiosk's menu.
- [ ] Taps work after the screen wakes, with no stuck touches (the PF1007L issue from
      2026-10-16).
- [ ] Parent nav rail taps leave Photos. **Exit full screen** above the iframe works.
- [ ] The two-finger hold or pull-down still opens the native kiosk menu over the iframe.
- [ ] No Kiosk link can navigate the top-level page (try "more info" if enabled, and a
      `_blank` link).
- [ ] Kiosk's fullscreen key or button does nothing harmful.
- [ ] With the `frontend/kiosk/` APK: from the iframe's devtools console, confirm whether
      `window.HomeHQKiosk` exists. If it does, don't ship until the bridge is origin-restricted.

**Alerts and idle**

- [ ] A test HA alert shows a toast or sheet above the iframe, and its buttons take taps.
- [ ] A navigating alert (doorbell → camera) leaves Photos, and the iframe unloads.
- [ ] Night and off schedules trigger while Photos (Kiosk) is shown. Wake restores the slideshow.
- [ ] Frame check-ins continue throughout (Home HQ logs).

**Photos and metadata**

- [ ] Landscape, portrait and no-EXIF photos in `single`/`contain`, `splitview` and `cover`.
      Screenshot each.
- [ ] Date and location are legible on a bright, a dark and a busy photo with `theme: solid` and
      `font_size: 120`. Screenshot each, and compare with the same photos in the custom viewer.
- [ ] A photo missing EXIF shows nothing, not wrong data. Confirm in the Immich UI.
- [ ] #7: tapping the Home photo opens Photos. Note which photo appears (expected: a different
      one).

**Keys and sleep**

- [ ] (After Phase 2, on staging first) Disconnect in Home HQ: Kiosk stops showing new photos
      within one slide. The flush clears cached ones. Kiosk doesn't crash.
- [ ] (After Phase 2) Let a short-TTL grant expire: same result, with no broker call needed.
- [ ] The frame's devtools show no `x-api-key`, gateway token or Kiosk password in any request,
      cookie, storage or page source.
- [ ] One-hour soak in Photos: `dumpsys meminfo com.wesbos.homehq` stays flat, and the WebView
      doesn't reload or crash.
- [ ] The overnight off period with Photos open: the screen goes dark, the iframe unloads, and
      the morning wake works.

**Owner decisions after the spike**

- [ ] Are Kiosk's extra sources and presentation worth the gateway, hostname and certificate
      work?
- [ ] Is #7 acceptable as an upstream dependency, or is it a blocker?
- [ ] What is the Immich server version? It must be ≥ v3.2.0 for Kiosk. It also decides
      whether PR #1's metadata-search adapter needs updating anyway.
