# PRD 002: Home HQ Kiosk (self-hosted family dashboard)

**Status:** Proposed
**Date:** 2026-11-05
**Related:** RFC 004 (secrets management), RFC 013 (storage tiering), `router/playbook.yml` (IoT isolation), sambartle/photo-frame-dashboard (upstream, MIT)
**Triage:** needs-triage

> Delivery plan agreed on 2026-11-05: the first delivery is the look (the unmodified frontend)
> with dummy data. Each vertical below (store, bins, calendar, weather, HA gateway, frames
> monitor, capability flags, then the integrations) ships as its own delivery. The infra parts
> (Flux component, router rules, frame runbook) live in the internal infra repo.

## Problem Statement

We want a wall-mounted family dashboard on a PF1007L photo frame (Allwinner A33, Android 6,
WebView 106, root ADB). It should show the day at a glance: what's next, weather, bins, who is
home, the shopping list, notes, photos and Home Assistant controls. sambartle's "Home HQ" fork of
photo-frame-dashboard has exactly this UX, but its backend is built entirely on Cloudflare:

- a Worker serves the page and the API
- D1 holds the data
- R2 holds the photos
- a Durable Object pushes alerts
- a Cloudflare Tunnel reaches Home Assistant (HA)
- scripts push HA state up to the Worker to stay inside free-plan CPU and request limits

All of that is redundant here. The homelab already runs HA, Immich, Mealie, SQLite-friendly
storage, a secrets story, cert-manager, and Flux/Woodpecker/Renovate. Much of upstream's complexity
exists only to fit the Cloudflare free plan. Running upstream as-is would also put family data
(notes, shopping, calendar, camera snapshots) on a third-party edge for no benefit.

The device is also hostile to modern defaults. It is an Android 6 device on the isolated IoT
Wi-Fi, and its trust store predates the current Let's Encrypt roots.

## Solution

Keep upstream's frontend. Replace its Cloudflare backend with a small self-hosted Rust service
that speaks the same `/api` contract, so the frontend runs essentially unmodified. Every external
data source sits behind a narrow interface (a "port"). Version 1 ships with simple default
implementations (SQLite, ICS, or "not configured"). Real integrations are added later without
touching the API layer:

- Immich for photos
- Mealie for shopping
- CalDAV for calendars
- Waze-via-HA or OSRM for travel times

The service lives in its own Forgejo repo, Woodpecker builds a single image (Vite build embedded
in the Rust binary), and Flux deploys it from this repo. The frame sits on the IoT Wi-Fi with a
fixed lease. It can reach only the dashboard over HTTPS, and it has no internet access. HTTPS
uses the existing Traefik wildcard certificate, made acceptable to Android 6 by adding the ISRG
roots to the frame's system trust store over root ADB.

> **Update 2026-11-05.** Two decisions below changed at deployment. **TLS:** a per-host Actalis
> certificate for `home-hq.prdv.cloud`, because the frame's stock store already has Actalis
> Authentication Root CA, so `/system` stays untouched (ISRG roots are now the fallback). **Network:**
> the dedicated-IP fallback is the design: the app terminates TLS itself on a MetalLB address
> (`lanService`), the Ingress is off, and the frame's one firewall rule reaches only that IP:443.

## User Stories

1. As a household member, I want the frame to show "next up" events for today, so that I know what's coming without opening my phone.
2. As a household member, I want calendar views for today, 3 days, a week and a month, so that I can plan ahead from the wall.
3. As a household member, I want events from several calendars, each attributed to the right person and colour, so that I can tell whose event is whose.
4. As a household member, I want recurring events (with exceptions and timezones) shown correctly, so that the dashboard can be trusted.
5. As a household member, I want current weather and a short forecast, so that I can dress and plan accordingly.
6. As a household member, I want the bins card to say which bins go out and to highlight from 4 PM the day before, so that we never miss a collection.
7. As a household member, I want to tap "They're out" on the bins card, so that the reminder clears for everyone.
8. As a household member, I want bank-holiday and other moved collections respected, so that the bins card is right in holiday weeks.
9. As a household member, I want to see who is home, so that I know without asking.
10. As a household member, I want a shared shopping list I can add to and tick off from the frame and my phone, so that whoever shops has the current list.
11. As a household member, I want my phone to see shopping changes within about 30 seconds, so that we don't buy things twice.
12. As a household member, I want to leave a short note for one person or everyone, so that I can pass messages on at home.
13. As a household member, I want a note optionally pushed to the recipient's phone through HA, so that urgent notes are seen.
14. As a household member, I want a photo slideshow when the frame is idle, so that it doubles as a photo frame.
15. As a household member, I want the photo slideshow to come from Immich (favorites, an album, "on this day"), so that I don't upload photos twice.
16. As a household member, I want to control lights, switches, scenes, covers and the thermostat from the frame, so that I don't need my phone for common actions.
17. As a household member, I want camera snapshots on the Home tab, so that I can glance at the door or garden.
18. As a household member, I want smoke, CO, water-leak and similar alerts to take over the screen with sound within seconds, so that the frame is a useful alarm surface.
19. As a household member, I want a "leave by" countdown for upcoming events that have a location, so that I leave on time.
20. As a household member, I want the travel-time feature to be optional and to fall back to the event's "Travel:" text, so that a missing provider never breaks the calendar.
21. As a household member, I want a phone page at the same address, so that I can use shopping, notes and calendar away from the frame.
22. As a household member, I want tabs whose data source is not configured to be hidden, so that the frame never shows empty or broken screens.
23. As the operator, I want to be notified through HA when the frame has been silent for 15 minutes, so that I notice a crashed or unplugged frame.
24. As the operator, I want the frame to pick up new dashboard versions automatically, so that I never touch the device after a deploy.
25. As the operator, I want to add an Immich, Mealie, CalDAV or travel-time integration by adding one adapter and configuration, so that integrations stay isolated and incremental.
26. As the operator, I want every adapter of a given port to pass the same behavioural test suite, so that swapping SQLite for Mealie (for example) can't silently change behaviour.
27. As the operator, I want the dashboard deployed by Flux from this repo with the image tag bumped by Renovate, so that it follows the same GitOps flow as everything else.
28. As the operator, I want secrets (dashboard key, HA token, ICS URLs, adapter credentials) in Kubernetes Secrets in line with RFC 004, so that no secret lives in git or in the image.
29. As the operator, I want household settings (people, bins rotation, stores, location) as reviewable configuration rather than baked into the image, so that changing a bin day doesn't need a rebuild.
30. As the operator, I want state (notes, bin status, frame check-ins, the default shopping list) on a small PVC, so that restarts and upgrades lose nothing.
31. As the operator, I want the backend to talk to HA in-cluster with a non-admin token, so that no tunnel or public exposure of HA is needed.
32. As the operator, I want HA alerts consumed through HA's WebSocket subscription, so that HA needs no extra automations or push scripts to feed the dashboard.
33. As the operator, I want the frame isolated on the IoT Wi-Fi, reachable only on its fixed lease and allowed only to the dashboard on 443, so that a rooted, root-ADB device can't reach the rest of the LAN.
34. As the operator, I want the frame blocked from the internet, so that the device's vendor firmware and apps can't phone home.
35. As the operator, I want the frame to validate the dashboard's TLS certificate, so that the access key is never sent over an untrusted connection.
36. As the operator, I want access controlled by upstream's shared-key-to-cookie flow, so that the frame and family phones work without accounts.
37. As the operator, I want the dashboard's health and errors visible in the existing Prometheus/Loki stack, so that it is monitored like everything else.
38. As the operator, I want a documented one-time frame setup (trust-store push, kiosk URL, screen-off mode), so that the frame can be rebuilt from scratch.

## Implementation Decisions

**Stack.** Rust, with an async HTTP framework (axum), a WebSocket server, SQLite, and an HTTP client.
Rust was chosen over Go and TypeScript on preference. Reusing upstream backend code had little
value once every data source sits behind a port. Accepted costs:

- Async traits used with dynamic dispatch need boxing or a helper crate.
- CalDAV client libraries are less mature, so the future CalDAV adapter may need to write some WebDAV REPORT queries by hand.
- CI needs a cargo cache to keep builds fast.

**Repo layout across forges.** The app lives in its own Forgejo repo. Woodpecker builds one
distroless image per tag. The upstream `dashboard/` frontend is vendored with a git subtree so
upstream fixes can be pulled. The Vite build output is embedded in the binary. This infra repo
holds only the deployment, the router rules and the frame runbook.

**API compatibility as the contract.** The backend reproduces upstream's routes and response shapes:

- `state`, `calendar`, `weather`
- `events`
- `notes`, `groceries` (including clear)
- `bins/out`
- `frames` (check-in and list)
- `alerts` (poll and WebSocket)
- `photos`
- `home` and `home/energy`
- `ha/camera` and `ha/art`
- the static pages: wall, phone and upload

It also keeps upstream's authentication: `?key=` on a page is swapped for a long-lived HttpOnly
cookie, `x-hq-key` is accepted on API calls, and keys are compared in constant time. Frontend
patches are kept minimal and listed in the app repo. The one planned patch hides tabs when
`/api/state` reports their capability as unavailable.

**Ports (deep modules with small interfaces).**

- `PhotoSource`: list photos (with metadata such as date and favorite flag) and fetch an image at
  a requested size. V1: not configured (Photos tab hidden). Next: Immich (favorites, an album,
  memories / "on this day"). Upstream's upload page and R2 storage are dropped.
- `ShoppingList`: list, add, tick and untick, clear done items. V1: SQLite. Next: Mealie.
- `CalendarSource`: list events in a time range, attributed to people. Adding and removing
  events is an optional capability. V1: read-only ICS feeds. Next: CalDAV (read and write).
- `TravelTime`: minutes from an origin (coordinates, or a "home" zone) to a free-text
  destination, or "unknown". V1: not configured (the frontend falls back to the event's "Travel:"
  text). Candidates for later: HA's Waze Travel Time action (live traffic, no key, unofficial
  API) or self-hosted OSRM/Valhalla plus a geocoder (no traffic). Upstream's caching and daily
  budget (results kept 10 minutes, a daily cap, only events within the next 4 hours) move into a
  shared wrapper around any implementation.
- Each port reports whether it is configured. The `/api/state` response aggregates these into
  capability flags.

**Other modules.**

- *Calendar engine*: fetch, parse, expand recurrences and exceptions, normalise timezones,
  attribute events to people, cache with a TTL. It is pure apart from fetching, which is injected.
- *Bins schedule*: the rotation (types, every N weeks from a start date), moved dates, and the
  highlight window from 4 PM the day before until marked out. Pure, with an injected clock.
- *HA gateway*: a reduced state snapshot (only the entities named in configuration), action calls,
  a camera and artwork proxy that keeps the last good image, and a WebSocket subscription that
  feeds alerts. It connects to HA through its in-cluster Service using a dedicated non-admin
  token. Upstream's push endpoints for HA state and alerts are not implemented.
- *Alert hub*: forwards alerts in process to connected frame WebSockets, with a sequence number for
  the polling fallback.
- *Frames monitor*: records check-ins, runs a periodic check, and sends an HA notification when a
  frame has been silent for 15 minutes or more.
- *Weather*: Open-Meteo for the configured latitude and longitude, cached. Met Office warnings
  are dropped as UK-only.
- *Store*: SQLite with embedded migrations. Tables: notes, bin state, frames, default shopping
  list, cache.
- *Config*: household settings in upstream's settings format (people, bins, stores, catalog,
  display), mounted from a ConfigMap, so changes need a restart rather than a rebuild. Secrets come
  from environment variables backed by a Secret.

**Deployment (this repo).** A new `home-hq` component, flat like the other components until
PRD 001's reorganisation lands. It contains:

- a single-replica Deployment using the Recreate strategy, since SQLite allows only one writer
- a 1Gi PVC
- a Service
- an Ingress on `home-hq.prdv.cloud` using the existing `lab.prdv.cloud` wildcard certificate
- the settings ConfigMap
- the Secret, documented in the README bootstrap section
- a ServiceMonitor, if the app exposes metrics

Renovate tracks the image.

**Network.** The router playbook gains:

- a fixed DHCP lease for the frame
- an IoT→LAN rule allowing only the frame's IP to reach Traefik's LoadBalancer IP on TCP 443
  (superseded: the dashboard's own LAN IP, see the update at the top)
- a "block WAN" rule for the frame's MAC, following the RM4 and SLZB pattern

DNS for the frame already goes through Pi-hole. Accepted trade-off: because the rule targets
Traefik's IP, the frame can reach other `*.lab` ingress hostnames. Each of those keeps its own
authentication, and the frame is the only allowed IoT client. If that becomes unacceptable, the
fallback is a dedicated MetalLB IP for the dashboard with the app terminating TLS itself (the
MQTT pattern).

**TLS on Android 6.** The live chain is `lab.prdv.cloud` → Let's Encrypt YR2 → ISRG Root YR →
cross-signed by ISRG Root X1. Android 6 trusts neither root. Decision: push ISRG Root X1 and ISRG
Root YR into the frame's system CA store over root ADB, once, as part of the frame runbook. The
existing Actalis issuer (used for the Roborock) is the fallback if the device can't be modified.
No Actalis certificate is issued for this. (Superseded: Actalis is now the primary, see the update
at the top.)

**Frame side.** Upstream's kiosk APK (full-screen WebView, two-finger menu, screen-off modes) and
its set-kiosk-URL script are reused, pointing at the dashboard hostname with the key. The frame
uses the `backlight` screen-off mode, which upstream uses on the A33.

## Testing Decisions

A good test exercises a module only through its public interface and asserts observable
behaviour, such as API responses, events returned for a range, or alerts delivered. Tests should
not reach into internals or snapshot private structures, and they use injected clocks and fakes
instead of real time or the network.

- **Calendar engine**: table-driven tests on ICS fixtures covering recurring rules, exception
  dates, edited instances, all-day events, daylight-saving transitions and person attribution.
- **Bins schedule**: rotations across year boundaries, moved dates, the 4 PM window, and the "out"
  state resetting after collection.
- **Port contract suites**: one shared behavioural test suite per port, run against every
  adapter. In v1 that means the SQLite `ShoppingList` (against a temporary database) and the ICS
  `CalendarSource`. The Mealie, Immich and CalDAV adapters must pass the same suites, against
  recorded or containerised fakes, when they are added. The travel-time caching and budget wrapper
  is tested against a counting fake.
- **API contract**: route-level tests against the full router, using fake ports. They cover:
  - response shapes, compared with fixtures recorded from upstream
  - the key → cookie redirect, cookie and header authentication, and rejection without a key
  - capability flags in `/api/state`
  - the alert WebSocket and polling fallback
- **HA gateway**: against a fake HA (REST plus WebSocket). Covers state reduction to the
  configured entities, action calls, camera proxy behaviour when HA fails (last good image), and
  alert subscription and reconnection.
- **Frames monitor**: check-in timing and the offline notification, with an injected clock and a
  fake notifier.
- **Infra**: `kustomize build` and `flux-local`-style validation of the new component, as for any
  manifest change. A manual check on the device confirms the certificate validates after the
  trust-store push.

There is no prior art for Rust tests in this repo. The new repo sets the conventions: `cargo
test` in Woodpecker for every push, with fixtures kept next to the tests.

## Out of Scope

- Immich, Mealie, CalDAV and travel-time adapters themselves. V1 defines the ports and contract
  tests only; each adapter is its own follow-up issue.
- Google Calendar write-back through a service account, and Google Routes.
- Met Office weather warnings.
- Upstream's photo upload page, Android share target and R2-style photo storage.
- Upstream's HA push scripts and push endpoints, replaced by the HA WebSocket subscription.
- Multiple frames with per-frame configuration. A single frame is assumed, though check-ins are
  stored per frame name.
- Exposing the dashboard outside the LAN. The phone page is reachable on the LAN and VPN only.
- Moving the IoT network to VLANs (see `router/firmware-upgrades.md`).
- Rooting or flashing procedures for the PF1007L beyond what the runbook needs.

## Further Notes

- Upstream is MIT-licensed (copyright Wes Bos, plus the fork's changes). The vendored frontend
  keeps its LICENSE, and the app repo is MIT-compatible.
- Upstream changes to the frontend's API usage are the main maintenance risk. The recorded
  response fixtures in the API contract tests should be refreshed whenever the subtree is updated.
- The frame runs root ADB without key checks (per upstream). The IoT isolation, the single
  443 rule and the WAN block are what make that acceptable. Do not relax those rules for
  convenience.
- Open question: whether HA's Waze Travel Time action fits the travel-time use (it is an
  unofficial API) or whether a self-hosted OSRM with a local geocoder is preferred. This is
  decided when that adapter is picked up.
