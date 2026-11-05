# `home-hq` component (draft for the Flux repo)

This folder is meant to be copied into the infra repo as the flat `home-hq` component, as PRD
002 describes. It deploys the dashboard image from GHCR with:

- a single-replica Deployment using the Recreate strategy
- a Service
- an Ingress on `home-hq.lab.prdv.cloud`, using the existing wildcard certificate
- the household settings as a ConfigMap

There is no PVC yet, because delivery 1 keeps everything in memory. There is no ServiceMonitor
yet, because the app doesn't expose `/metrics` yet.

## Bootstrap

1. **Image.** Push a `v*` tag in this repo (see the main README). While the GHCR package is
   private, create a pull secret and uncomment `imagePullSecrets` in `deployment.yaml`:

   ```bash
   kubectl -n home-hq create secret docker-registry ghcr-pull --docker-server=ghcr.io --docker-username=<github user> --docker-password=<PAT with read:packages>
   ```

   Alternatively, make the package public. It contains no secrets.
2. **Secret** `home-hq` in namespace `home-hq`, holding `HQ_KEY`, created the way RFC 004
   prescribes. The shape is in [`secret.example.yaml`](secret.example.yaml). Generate the key
   with:

   ```bash
   openssl rand -base64 32 | tr '+/' '-_'
   ```

   Use URL-safe characters, because the key goes in the kiosk URL as `?key=`.
3. **Settings.** Edit [`settings.json`](settings.json): timezone, location, people, bins and
   shopping. It holds no secrets. Changes roll the pod automatically, because the ConfigMap name
   is hashed.
4. **DNS.** `home-hq.lab.prdv.cloud` must resolve to Traefik's LoadBalancer IP. The wildcard
   record already covers it, if `*.lab` is a wildcard in Pi-hole.
5. **Renovate** picks up `ghcr.io/peddavid/home-kiosk` in `deployment.yaml` through its
   Kubernetes/Flux manager, if that manager is enabled for this path.

## Check

```bash
kustomize build .   # or flux-local, as for any manifest change
```

Then open:

```
https://home-hq.lab.prdv.cloud/?key=<HQ_KEY>
```

You should be redirected to `/` with a `hq` cookie set, and the dashboard should show dummy
data. `https://home-hq.lab.prdv.cloud/m` is the phone page.

## Later deliveries change

- A small PVC, once the first piece of dashboard-owned state lands (notes, bins or sessions).
- `HA_TOKEN`, calendar URLs and adapter credentials added to the Secret.
- A ServiceMonitor once `/metrics` exists.
- Keycloak client secrets, per the [auth RFC](../../docs/rfc-device-auth.md).
