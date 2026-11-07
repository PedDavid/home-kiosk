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
    # immich:
    #   publicUrl: https://photos.example.com # shows the Share button (QR to the photo in Immich)
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

1. **The `home-hq` namespace and Secret.** The Secret holds `HQ_KEY` (and optionally, with
   `calendars.existingSecret`, fixed calendars as `HQ_CALENDARS`; see [Calendars](#calendars)) and is
   created the way RFC 004 prescribes. Generate the key with:

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

Calendars are added on the dashboard's calendar page (**Status → Calendars: add or change**, or
`/calendars`). CalDAV servers connect by signing in on a phone (Nextcloud Login Flow v2, which
RustiCal also offers), and ICS links can be added too. With `persistence` on (the default) and
a dashboard key, they are kept encrypted on the PVC (`calendars.bin`). See the app README's
[Calendars](../../README.md#calendars).

To limit the servers the page accepts, set `HQ_CALENDAR_SERVERS` through `extraEnv`:

```yaml
  values:
    extraEnv:
      - name: HQ_CALENDAR_SERVERS
        value: https://dav.example.com
```

Calendars can also be fixed in the Secret as `HQ_CALENDARS` (`calendars.existingSecret`), a JSON
list of ICS links and CalDAV calendars, for setups that keep everything in git:

```bash
kubectl -n home-hq create secret generic home-hq \
  --from-literal=HQ_KEY=… \
  --from-file=HQ_CALENDARS=calendars.json
```

Calendars are read every 10 minutes. One that fails keeps its last events, and the frame's Status
panel shows "<name>: …" (never the link or password).

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
| `calendars.existingSecret` / `calendars.secretKey` | `""` / `HQ_CALENDARS` | Optional Secret with fixed calendars (ICS links, CalDAV URLs and passwords). Calendars are normally added on the calendar page |
| `settings` | `{}` | Household settings. Empty means the bundled example |
| `immich.publicUrl` | `""` | `HQ_IMMICH_PUBLIC_URL`: the Immich address phones open from the Photos tab's Share QR code. Empty hides the Share button |
| `ingress.enabled`, `className`, `host`, `annotations` | `true`, `traefik`, `home-hq.example.com`, `{}` | Ingress |
| `ingress.tls.enabled` / `ingress.tls.secretName` | `true` / `""` | TLS. Empty means Traefik's default certificate |
| `tls.existingSecret` | `""` | `kubernetes.io/tls` Secret. Set: the app also serves TLS on 8443, reloading renewed certificates |
| `lanService.enabled`, `type`, `port`, `externalTrafficPolicy`, `annotations` | `false`, `LoadBalancer`, `443`, `Local`, `{}` | A Service straight to the TLS listener: a dedicated LAN address. Needs `tls.existingSecret` |
| `weather.provider` | `open-meteo` | `open-meteo` serves the real forecast for `settings.location` (needs egress to `api.open-meteo.com`). `demo` serves the generated demo forecast and makes no calls |
| `weather.openMeteoUrl` | `""` | Forecast endpoint for a self-hosted Open-Meteo. Empty means `https://api.open-meteo.com/v1/forecast` |
| `persistence.enabled`, `existingClaim`, `storageClass`, `accessMode`, `size`, `mountPath`, `annotations` | `true`, `""`, `""` (cluster default), `ReadWriteOnce`, `16Mi`, `/data`, `{}` | A PVC for state that must survive restarts: the encrypted Immich device grant. Only created with `key.existingSecret` |
| `extraEnv` | `[]` | Extra env vars (HA token and adapter credentials in later deliveries) |
| `logLevel` | `info` | `RUST_LOG` |
| `resources`, `podSecurityContext`, `securityContext`, `nodeSelector`, `tolerations`, `affinity`, `podAnnotations` | small, non-root, read-only root filesystem | The usual |

Later deliveries add more dashboard-owned state to the `persistence` volume, a ServiceMonitor
(once `/metrics` exists), and Keycloak client settings ([auth RFC](../../docs/rfc-device-auth.md)).
