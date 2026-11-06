# `home-hq` Helm chart

This chart deploys the Home HQ dashboard: a single-replica Deployment with the Recreate
strategy, a Service, an Ingress, and the household settings as a ConfigMap. Changing the settings
rolls the pod. Optionally the app terminates TLS itself behind a dedicated LAN address (see
[Dedicated LAN address](#dedicated-lan-address)), and keeps the Immich pairing on a small volume
(see [Persistence](#persistence)).

Each `v*` tag in this repo publishes the chart and the image with the same version:

- chart `oci://ghcr.io/peddavid/charts/home-hq:<version>`
- image `ghcr.io/peddavid/home-kiosk:<version>`

The chart runs its matching image by default, so bumping the chart version upgrades the app.

## Flux

The Flux repo needs three things: a source, a `HelmRelease` with your values, and the Secret.

```yaml
apiVersion: source.toolkit.fluxcd.io/v1
kind: OCIRepository
metadata:
  name: home-hq
  namespace: home-hq
spec:
  interval: 1h
  url: oci://ghcr.io/peddavid/charts/home-hq
  ref:
    tag: 0.1.0 # Renovate bumps this
  layerSelector:
    mediaType: application/vnd.cncf.helm.chart.content.v1.tar+gzip
    operation: copy
  # secretRef:            # while the GHCR package is private
  #   name: ghcr-pull
---
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: home-hq
  namespace: home-hq
spec:
  interval: 1h
  chartRef:
    kind: OCIRepository
    name: home-hq
  values:
    # imagePullSecrets: [{ name: ghcr-pull }]   # while the image is private
    key:
      existingSecret: home-hq # holds HQ_KEY, created per RFC 004
    ingress:
      host: home-hq.prdv.cloud
    settings:
      timezone: Europe/Lisbon
      location: { lat: 38.7223, lon: -9.1393, units: celsius }
      people:
        - { id: alex, name: Alex, color: "#2D5BFF", ha: person.alex }
        - { id: sam, name: Sam, color: "#E4007C", ha: person.sam }
      bins:
        types:
          general: { name: Black, what: General waste, color: "#222222", lid: "#222222" }
          recycling: { name: Yellow, what: Plastic and metal, color: "#F2C200" }
        every:
          - { bin: general, from: "2026-01-06", weeks: 1 }
          - { bin: recycling, from: "2026-01-08", weeks: 2 }
      shopping:
        stores: [{ id: grocer, name: Supermarket }]
        categories: [Produce, Bakery, Dairy & Eggs, Pantry, Household]
        usuals: [Milk, Eggs, Bread]
        catalog:
          Produce: [Apples, Bananas, Tomatoes]
          Pantry: [Coffee beans, Pasta, Rice]
```

`settings` takes upstream Home HQ's `settings.json` format written as YAML. A complete example is
in [`config/settings.example.json`](../../config/settings.example.json). Leave `settings` out to
run with that bundled example. Quote dates such as `"2026-01-06"` so YAML keeps them as strings.

## Bootstrap

1. **The `home-hq` namespace and Secret.** The Secret holds `HQ_KEY` and is created the way RFC
   004 prescribes. Generate the key with:

   ```bash
   openssl rand -base64 32 | tr '+/' '-_'
   ```

   Use URL-safe characters, because the key goes in the kiosk URL as `?key=`.
2. **While the GHCR packages are private**, create a pull secret in the namespace and reference it
   from both the `OCIRepository` (`secretRef`) and the values (`imagePullSecrets`):

   ```bash
   kubectl -n home-hq create secret docker-registry ghcr-pull --docker-server=ghcr.io --docker-username=<github user> --docker-password=<PAT with read:packages>
   ```

   Alternatively, make both packages public (`home-kiosk` and `charts/home-hq`). Neither contains
   secrets.
3. **DNS.** `home-hq.prdv.cloud` must resolve to Traefik's LoadBalancer IP (or to the LAN
   address, with `lanService`).
4. **TLS.** By default the Ingress uses Traefik's default certificate. If your other apps
   reference the wildcard Secret explicitly, set `ingress.tls.secretName`.

Then open:

```
https://home-hq.prdv.cloud/?key=<HQ_KEY>
```

You should be redirected to `/` with a `hq` cookie set, and the dashboard should show dummy
data. `/m` is the phone page.

## Calendars

"Next up", the people lanes and the calendar views read private iCal links, one or more per
person. The links are secrets (anyone holding one can read that calendar), so they live in the
Secret as `HQ_CALENDARS`, a JSON list:

```json
[
  { "url": "https://calendar.google.com/calendar/ical/…/private-…/basic.ics", "people": ["alex"] },
  { "url": "https://calendar.google.com/calendar/ical/…/private-…/basic.ics", "people": ["sam"] },
  { "url": "webcal://p01-caldav.icloud.com/published/2/…", "people": "all" }
]
```

`people` names person ids from `settings.people` (a list, one id, or `"all"`). In Google
Calendar the link is *Settings → (the calendar) → Integrate calendar → Secret address in iCal
format*. Add it to the Secret and point the chart at it:

```bash
kubectl -n home-hq create secret generic home-hq \
  --from-literal=HQ_KEY=… \
  --from-file=HQ_CALENDARS=calendars.json
```

```yaml
  values:
    calendars:
      existingSecret: home-hq
```

Feeds are read every 10 minutes. A feed that fails keeps its last events, and the frame's Status
panel shows "Calendar N: …" (never the link). The calendar is read-only: the frame's "+" is
refused until a writable source (CalDAV) is configured.

## Travel times

"Leave by" on "Next up" and in the event sheet counts back from the start by the drive time. By
default that's the event's `Travel: 15` line. With `settings.travel` set, timed events with a
location starting in the next 4 hours get a drive time from a self-hosted router instead
(`drive: { mins, from }` in `/api/calendar`), and `Travel:` stays the fallback whenever the
router can't answer. The URLs aren't secrets, so this lives in the settings:

```yaml
  values:
    settings:
      location: { lat: 38.7223, lon: -9.1393 }
      people:
        - { id: alex, name: Alex, color: "#2D5BFF" }
        # Optional: Sam's drives start from work instead of home.
        - { id: sam, name: Sam, color: "#E4007C", origin: "Rua Augusta 1, Lisboa" }
      travel:
        osrm: http://osrm.routing.svc.cluster.local:5000
        photon: http://photon.routing.svc.cluster.local:2322   # or nominatim: http://…
        home: { lat: 38.7223, lon: -9.1393 }                 # optional, defaults to location
        dailyCap: 300                                        # optional
```

`home` and a person's `origin` are coordinates or an address (geocoded). With several people at
one event the longest drive counts. Answers, failures too, are kept 10 minutes, and at most
`dailyCap` routes are asked per day; after that `Travel:` is used until midnight. There is no
live traffic: OSRM's times come from OpenStreetMap road speeds.

### Running OSRM and Photon

Neither ships with this chart; run them next to it (a `routing` namespace, say). Both need the
data prepared once, which is the heavy part.

**OSRM** ([osrm-backend](https://github.com/Project-OSRM/osrm-backend), see its
[Using Docker](https://github.com/Project-OSRM/osrm-backend#using-docker) section), for a country
extract from [Geofabrik](https://download.geofabrik.de/) (Portugal is about 350 MB; preparing it
wants roughly 2–4 GB of RAM once, and serving it a few hundred MB):

```bash
wget https://download.geofabrik.de/europe/portugal-latest.osm.pbf
docker run --rm -v "$PWD:/data" ghcr.io/project-osrm/osrm-backend osrm-extract -p /opt/car.lua /data/portugal-latest.osm.pbf
docker run --rm -v "$PWD:/data" ghcr.io/project-osrm/osrm-backend osrm-partition /data/portugal-latest.osrm
docker run --rm -v "$PWD:/data" ghcr.io/project-osrm/osrm-backend osrm-customize /data/portugal-latest.osrm
# Serve (in the cluster: a Deployment with the files on a PVC, a Service on 5000):
docker run -p 5000:5000 -v "$PWD:/data" ghcr.io/project-osrm/osrm-backend osrm-routed --algorithm mld /data/portugal-latest.osrm
```

Refresh the extract every few months (a CronJob that re-runs the three steps and restarts the
Deployment works). Check it with
`curl "http://osrm:5000/route/v1/driving/-9.1393,38.7223;-9.3817,38.8029?overview=false"`.

**Photon** ([komoot/photon](https://github.com/komoot/photon)) is the lighter geocoder: a Java
service with a prebuilt index, no database to run. Its README explains downloading the prebuilt
indexes from [GraphHopper's download server](https://download1.graphhopper.com/public/) (the
planet, or smaller extracts), then running `java -jar photon.jar` (port 2322). The unofficial
[rtuszik/photon-docker](https://github.com/rtuszik/photon-docker) image does the download for
you: set `REGION` to a country code (e.g. `pt`), a region name, or `planet`. Check it with
`curl "http://photon:2322/api?q=Sintra&limit=1"`.

**Nominatim** instead of Photon (`travel.nominatim`) is also supported, for example with
[mediagis/nominatim-docker](https://github.com/mediagis/nominatim-docker) (`mediagis/nominatim`,
with `PBF_URL` set to the same Geofabrik extract). It needs PostgreSQL and much more disk and
import time; prefer it only if you already run one. The public `nominatim.openstreetmap.org`
works for a try-out (the daily cap keeps the load low), but its
[usage policy](https://operations.osmfoundation.org/policies/nominatim/) is not meant for an
always-on dashboard.

#### Reference

| Project | Setup | What this app calls |
|---|---|---|
| OSRM | [osrm-backend: Using Docker](https://github.com/Project-OSRM/osrm-backend#using-docker); data from [Geofabrik](https://download.geofabrik.de/) | [`route` service](https://project-osrm.org/docs/v5.24.0/api/#route-service): `GET /route/v1/driving/{lon},{lat};{lon},{lat}?overview=false` |
| Photon | [komoot/photon](https://github.com/komoot/photon) (install, prebuilt indexes); [rtuszik/photon-docker](https://github.com/rtuszik/photon-docker) | `GET /api?q=…&limit=1&lat=…&lon=…` (search with a location bias, in the Photon README) |
| Nominatim | [Installation](https://nominatim.org/release-docs/latest/admin/Installation/); [mediagis/nominatim-docker](https://github.com/mediagis/nominatim-docker) | [`/search`](https://nominatim.org/release-docs/latest/api/Search/): `GET /search?q=…&format=jsonv2&limit=1&viewbox=…` |

The frame never talks to either: only the pod does, so the frame's firewall rule doesn't change.

## Dedicated LAN address

Behind Traefik, a device allowed to reach Traefik's IP can reach every other ingress by sending a
different hostname. For a kiosk on an untrusted network, give the dashboard an address of its own
instead. The app serves TLS from a `kubernetes.io/tls` Secret, and `lanService` points a
LoadBalancer straight at it, so a firewall rule for that IP and port reaches the dashboard and
nothing else:

```yaml
  values:
    tls:
      existingSecret: home-hq.prdv.cloud-tls # e.g. from cert-manager
    lanService:
      enabled: true
      annotations:
        metallb.io/loadBalancerIPs: 192.168.8.56
        external-dns.kubernetes.io/hostname: home-hq.prdv.cloud
    ingress:
      enabled: false # so DNS points only at the LAN address
```

The whole Secret is mounted (no `subPath`) and the app re-reads it every minute, so a renewed
certificate is served without a restart. A file that doesn't load is logged and the previous
certificate stays in use. Plain HTTP on 8080 stays on for the probes. `externalTrafficPolicy:
Local` (the default here) keeps client addresses in the logs.

The listener is rustls: TLS 1.2 and 1.3, AEAD suites only (AES-GCM, ChaCha20). Android 6 with
WebView 106 negotiates TLS 1.2 ECDHE-RSA-AES-GCM; use an RSA certificate if a client is that old.

## Persistence

With Immich QR pairing (`HQ_IMMICH_DEVICE_AUTH_URL` in `extraEnv`), the issued key lives in
memory unless the chart gives the app a volume, so every restart or deploy means pairing the
frame again, and the old key lingers in Immich until the broker revokes it at expiry.
`persistence` keeps it, and is **on by default** whenever `key.existingSecret` is set:

```yaml
  values:
    key:
      existingSecret: home-hq # required: the grant is encrypted with a key derived from HQ_KEY
    # persistence:            # on by default; only needed to change or turn it off
    #   storageClass: local-path   # empty uses the cluster default (local-path on k3s)
    #   enabled: false             # keep the grant in memory only, as before 0.3.0
    extraEnv:
      - { name: HQ_IMMICH_URL, value: https://photos.example.com/api }
      - { name: HQ_IMMICH_DEVICE_AUTH_URL, value: https://device.photos.example.com }
```

With a key Secret, this creates a `ReadWriteOnce` PVC (`<release>-data`, 16Mi; or set `existingClaim`), mounts it at
`/data` and sets `HQ_STATE_DIR`. The root filesystem stays read-only. The app stores the issued
key, its expiry and scope in `/data/immich-grant.bin`, encrypted with AES-256-GCM under a key
derived from `HQ_KEY` (HKDF-SHA256) and bound to the configured broker and Immich URLs.

- Without `key.existingSecret` no PVC is created, since the app would write nothing without
  `HQ_KEY`. Pairing then behaves as with persistence off, and NOTES says so.
- The PVC needs a default StorageClass, or set `persistence.storageClass` or `existingClaim`.
  Otherwise the claim, and so the pod, stays Pending. Set `persistence.enabled: false` to run
  without a volume.
- local-path volumes live on one node, so the pod then only schedules on that node.
- On start, a grant that has expired, or that can't be decrypted because `HQ_KEY` or one of the
  URLs changed, is deleted and the frame shows "disconnected". Rotating `HQ_KEY` therefore means
  pairing again; the old key expires at the broker as before.
- **Disconnect** asks the broker to revoke the key, then deletes the file. The broker deletes
  the key in Immich itself (retrying if Immich is down). If the broker can't be reached, the grant
  and the file are kept so Disconnect can be retried.

When `persistence` is on and `podSecurityContext` has no `fsGroup`, the chart sets it to the
run-as group, so provisioners that honour `fsGroup` give the non-root user a writable volume.
local-path doesn't need it (its directories are world-writable). The Recreate strategy stops the
old pod before the new one mounts the claim, which a single-node `ReadWriteOnce` volume needs.

## Values

| Value | Default | Meaning |
|---|---|---|
| `image.repository` / `image.tag` | `ghcr.io/peddavid/home-kiosk` / chart `appVersion` | Image |
| `imagePullSecrets` | `[]` | For private GHCR |
| `key.existingSecret` / `key.secretKey` | `""` / `HQ_KEY` | Secret holding the dashboard key. Unset means the dashboard is open |
| `calendars.existingSecret` / `calendars.secretKey` | `""` / `HQ_CALENDARS` | Secret holding the calendar feeds ([Calendars](#calendars)). Unset means demo events |
| `settings` | `{}` | Household settings. Empty means the bundled example. `settings.travel` turns on drive times ([Travel times](#travel-times)) |
| `ingress.enabled`, `className`, `host`, `annotations` | `true`, `traefik`, `home-hq.example.com`, `{}` | Ingress |
| `ingress.tls.enabled` / `ingress.tls.secretName` | `true` / `""` | TLS. Empty means Traefik's default certificate |
| `tls.existingSecret` | `""` | `kubernetes.io/tls` Secret. Set: the app also serves TLS on 8443, reloading renewed certificates |
| `lanService.enabled`, `type`, `port`, `externalTrafficPolicy`, `annotations` | `false`, `LoadBalancer`, `443`, `Local`, `{}` | A Service straight to the TLS listener: a dedicated LAN address. Needs `tls.existingSecret` |
| `persistence.enabled`, `existingClaim`, `storageClass`, `accessMode`, `size`, `mountPath`, `annotations` | `true`, `""`, `""` (cluster default), `ReadWriteOnce`, `16Mi`, `/data`, `{}` | A PVC for state that must survive restarts: the encrypted Immich device grant. Only created with `key.existingSecret` |
| `extraEnv` | `[]` | Extra env vars (HA token and adapter credentials in later deliveries) |
| `logLevel` | `info` | `RUST_LOG` |
| `resources`, `podSecurityContext`, `securityContext`, `nodeSelector`, `tolerations`, `affinity`, `podAnnotations` | small, non-root, read-only root filesystem | The usual |

Later deliveries add more dashboard-owned state to the `persistence` volume, a ServiceMonitor
(once `/metrics` exists), and Keycloak client settings ([auth RFC](../../docs/rfc-device-auth.md)).
