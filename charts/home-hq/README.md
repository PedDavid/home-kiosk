# `home-hq` Helm chart

This chart deploys the Home HQ dashboard: a single-replica Deployment with the Recreate
strategy, a Service, an Ingress, and the household settings as a ConfigMap. Changing the settings
rolls the pod.

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
      host: home-hq.lab.prdv.cloud
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
3. **DNS.** `home-hq.lab.prdv.cloud` must resolve to Traefik's LoadBalancer IP.
4. **TLS.** By default the Ingress uses Traefik's default certificate. If your other apps
   reference the wildcard Secret explicitly, set `ingress.tls.secretName`.

Then open:

```
https://home-hq.lab.prdv.cloud/?key=<HQ_KEY>
```

You should be redirected to `/` with a `hq` cookie set, and the dashboard should show dummy
data. `/m` is the phone page.

## Values

| Value | Default | Meaning |
|---|---|---|
| `image.repository` / `image.tag` | `ghcr.io/peddavid/home-kiosk` / chart `appVersion` | Image |
| `imagePullSecrets` | `[]` | For private GHCR |
| `key.existingSecret` / `key.secretKey` | `""` / `HQ_KEY` | Secret holding the dashboard key. Unset means the dashboard is open |
| `settings` | `{}` | Household settings. Empty means the bundled example |
| `ingress.enabled`, `className`, `host`, `annotations` | `true`, `traefik`, `home-hq.example.com`, `{}` | Ingress |
| `ingress.tls.enabled` / `ingress.tls.secretName` | `true` / `""` | TLS. Empty means Traefik's default certificate |
| `extraEnv` | `[]` | Extra env vars (HA token and adapter credentials in later deliveries) |
| `logLevel` | `info` | `RUST_LOG` |
| `resources`, `podSecurityContext`, `securityContext`, `nodeSelector`, `tolerations`, `affinity`, `podAnnotations` | small, non-root, read-only root filesystem | The usual |

Later deliveries add a small PVC (dashboard-owned state), a ServiceMonitor (once `/metrics`
exists), and Keycloak client settings ([auth RFC](../../docs/rfc-device-auth.md)).
