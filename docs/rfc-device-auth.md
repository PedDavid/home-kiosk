# RFC: Frame pairing and step-up authentication

**Status:** Draft
**Number:** TBD (to be assigned in the infra repo's RFC sequence)
**Date:** 2026-11-05
**Related:** PRD 002 (Home HQ Kiosk, [`prd-002-home-hq-kiosk.md`](prd-002-home-hq-kiosk.md)), RFC 004 (secrets management), Keycloak, `router/playbook.yml` (IoT isolation)

## Summary

Replace the shared dashboard key with real identities:

- **Phones** sign in with Keycloak (OIDC authorization code flow with PKCE).
- **The frame** is paired once using the OAuth 2.0 Device Authorization Grant (RFC 8628)
  through Keycloak. It shows a QR code, an authorised household member approves it on their
  phone, and the frame gets its own revocable session. No secret is ever typed into or stored in
  the kiosk URL.
- **Sensitive home controls** (unlocking doors, opening the garage, disarming) need step-up
  authentication. Actions that make the house *safer* (locking, closing the water valve,
  turning things off) never do.

## Problem

PRD 002 v1 keeps upstream's access model: one shared key, put in the URL once as `?key=` and
then kept in an HttpOnly cookie. That works, but:

1. **One secret for everything.** The frame and every family phone hold the same key. A lost
   phone, or a key copied from the frame, can only be revoked by rotating the key and
   re-enrolling every device, including the frame. The frame is meant to be never touched.
2. **The key is easy to extract from the frame.** The PF1007L runs root ADB without key checks.
   Anyone who can reach it over ADB can read the kiosk URL, cookies and app storage. The IoT
   isolation limits who can reach it, but whatever credential the frame holds should be treated
   as exposed.
3. **No identity.** The backend can't tell who did something. There's no audit trail ("who
   unlocked the door?") and no per-person behaviour (a note's "from", whose phone to notify).
4. **All controls are equal.** Once the dashboard has the HA token and exposes locks (upstream
   has a hold-to-unlock button in the doorbell takeover), the shared key on a rooted wall device
   is enough to open the front door. HA has no per-entity permissions for non-admin users, so
   the dashboard is the only place this can be enforced.

## Goals

- The frame authenticates as its own **revocable, low-privilege principal**, set up once with no
  secret typed on the device and no secret in the URL.
- Each phone authenticates as a **person**, through the existing Keycloak.
- Revocation is per device and per person, and needs no change on other devices.
- Sensitive actions need a **fresh, strong proof of a person**, no matter which device sent them.
- The frame never talks to Keycloak directly. It keeps its single firewall rule (443 to the dashboard's
  own LAN address only, not Traefik).
- It works on Android 6 / WebView 106 with upstream's kiosk APK, unmodified.
- Every control action is logged with its principal (to Loki).

## Non-goals

- Making a credential on the frame impossible to extract. With root ADB this can't be done.
  What this RFC does is make extraction **limited** (low privilege), **visible** (audit) and
  **recoverable** (revocation).
- Per-entity permissions inside HA.
- Multiple households or guest accounts. These could be added later as Keycloak groups.

## Design

### Principals

The backend knows two kinds of principal, carried in its own session (an opaque random
256-bit id in an HttpOnly, Secure, SameSite=Lax cookie):

| Principal | Obtained by | Can do |
|---|---|---|
| `Person { sub, person_id }` | Keycloak login (phones, laptops) | Everything, tier 2 after step-up |
| `Device { id, name, approved_by }` | Device pairing (the frame) | Tiers 0 and 1, tier 2 only through step-up |

`person_id` comes from mapping the Keycloak user (`sub` or `preferred_username`) to a
`people[]` entry in settings, e.g. `"keycloak": "alex"`. A Keycloak user in the household group
who is mapped to no person can still sign in, but has no "from" for notes.

Access needs membership of a Keycloak group, e.g. `/household`, checked from the `groups`
claim. Approving a device pairing needs `/household-admin`.

### Phones: OIDC authorization code with PKCE

- A request with no valid session (an HTML page) redirects to Keycloak's authorization endpoint
  with client `home-hq` (confidential, secret from a Kubernetes Secret per RFC 004), scope
  `openid`, PKCE and `state`/`nonce`. API calls get `401 locked` as today, so the frontend's
  existing "Not connected" handling still applies.
- On the callback the backend exchanges the code, validates the ID token (issuer, audience,
  signature from JWKS, nonce), checks the group, and creates a `Person` session.
- Session lifetime follows the Keycloak SSO session. The backend refreshes it in the
  background and drops the session when Keycloak refuses (logout, revocation, user disabled).
- Phones reach Keycloak directly. They're on the LAN or VPN, as the dashboard requires anyway.

### The frame: device pairing via RFC 8628

The **backend** is the OAuth client and runs the device flow on the frame's behalf. The frame
only talks to the backend.

```
Frame (WebView)              Backend                         Keycloak               Phone (admin)
  | GET /?frame=kitchen        |                                 |                         |
  | GET /api/state ----------->| no session -> 401               |                         |
  | [pairing screen]           |                                 |                         |
  | POST /api/pair/start ----->| POST /device/auth (client       |                         |
  |                            |   home-hq-frame, scope          |                         |
  |                            |   openid offline_access) ------>|                         |
  |                            |<-- device_code, user_code,      |                         |
  |                            |    verification_uri_complete    |                         |
  |<-- pairing cookie, code, QR|                                 |                         |
  |                            |                                 |<-- scan QR, log in (MFA),
  |                            |                                 |    approve "Kitchen frame"
  |                            | poll POST /token (device_code) >|                         |
  |                            |<-- access, refresh (offline)    |                         |
  |                            | check approver in               |                         |
  |                            |   /household-admin              |                         |
  |                            | create Device session           |                         |
  | GET /api/pair/status ----->| -> paired, set session cookie   |                         |
  | reload, normal operation   |                                 |                         |
```

Details:

- **Starting a pairing** is unauthenticated, so it's rate-limited (a few per minute, a handful
  pending at once), and a pending pairing expires with Keycloak's `expires_in` (about 10
  minutes). A pending pairing grants nothing until a human approves it in Keycloak.
- **Approval** is an ordinary Keycloak login on the approver's phone, so it gets whatever MFA
  the realm enforces. The consent screen shows the device name from `?frame=` so the approver
  can tell what they're approving. The backend rejects the result unless the approving user is in
  `/household-admin`. Keycloak's device grant can't easily be limited by group on its own side.
- **The device's credential** is the Keycloak *offline* refresh token, kept **server-side only**,
  encrypted at rest with a key from the Kubernetes Secret. The frame holds only the opaque session
  cookie. An extracted cookie is a low-privilege session, revocable on its own, not a Keycloak
  credential.
- **Liveness and revocation**:
  - The backend refreshes each device's offline token every few hours. `invalid_grant` means
    the device was revoked: the session is deleted, the next API call gets 401, and the frame
    shows the pairing screen again.
  - If Keycloak is unreachable (network error, 5xx), the session stays valid for a grace period
    (e.g. 72 hours since the last successful refresh), so a Keycloak outage doesn't blank the
    wall.
  - Revoking is either removing the offline session in Keycloak (user → Sessions, or client →
    Sessions) or "Forget device" in the dashboard, which calls Keycloak's revocation endpoint
    and deletes the session.
- **Identity semantics:** tokens from a device grant belong to the approving *user*. The backend
  doesn't treat the frame as that user. It records `approved_by` for audit and gives the
  session `Device` privileges only.
- **Persistence:** device sessions have to survive restarts, or every deploy would unpair the
  frame. That brings back a small amount of state owned by the dashboard (see Open questions).

### Keycloak configuration (infra repo)

- Client `home-hq`: confidential, standard flow, redirect URI
  `https://home-hq.prdv.cloud/auth/callback`, a `groups` mapper, and PKCE required.
- Client `home-hq-frame`: confidential, **OAuth 2.0 Device Authorization Grant** enabled,
  standard flow off, `offline_access` allowed, consent required (so the approver sees what
  they approve).
- Realm or client: offline session idle timeout comfortably above the refresh interval (e.g. 30
  days), with no hard maximum, so a frame that's always on stays paired.
- Groups `/household` and `/household-admin`.
- Step-up: a browser flow with Level of Authentication conditions, e.g. LoA 1 = password,
  LoA 2 = WebAuthn or OTP, mapped to ACR values (`aal1`, `aal2`).

### Step-up for home controls

Actions are sorted by what they do to the house's exposure:

| Tier | Examples | Requirement |
|---|---|---|
| 0: read | State, calendar, cameras, presence | Any principal |
| 1: comfort, and actions that reduce exposure | Lights, scenes, media, blinds, thermostat within its range; **lock**, **close garage**, **close water valve**, turn off | Any principal |
| 2: actions that increase exposure | **Unlock**, **open garage or gate**, disarm an alarm, anything listed as sensitive in settings | Fresh step-up by a person |

Rules:

- **Tier membership is decided by the backend**, from the entity domain and the requested
  service (`lock.unlock`, `cover.open_cover` on a garage, `alarm_control_panel.alarm_disarm`),
  plus an explicit `sensitive: true` on a device in settings. It's never decided by the
  frontend.
- **Emergency actions stay tier 1.** Closing the water main during a leak alarm or locking a door
  must never be slowed by authentication.
- **Phones:** a tier 2 request without a fresh elevation gets `403 { "error": "step-up", ... }`.
  The phone page then redirects to Keycloak with `acr_values=aal2` and `max_age=0` (forced
  re-authentication). On return the backend checks `acr` and `auth_time` and grants a short
  elevation (e.g. 2 minutes) for that session only. The original action is retried.
- **The frame** has no person attached, so it has to borrow one. Options, in the order I
  recommend:
  1. **Off by default.** Tier 2 from a `Device` is refused, and the frame shows "Use your
     phone". This is the default until one of the options below exists.
  2. **Approve on phone.** The frame asks, the backend sends an HA actionable notification ("Unlock
     the front door from the Kitchen frame?") to the people allowed to approve, and executes when
     the action event arrives over the HA WebSocket subscription (which PRD 002 already has). This
     adds no new infrastructure, but it proves possession of a phone with the HA app, not a fresh
     Keycloak login.
  3. **Per-action QR code.** The frame shows a QR code, and scanning it opens a dashboard page on
     the phone. That page goes through Keycloak step-up (`aal2`) and approves this single pending
     action. This gives the strongest proof and reuses the pairing mechanics, but it's slow when
     someone is waiting at the door.
  4. **PIN pad on the frame.** It's fast, but a PIN typed on a rooted device with root ADB is
     only as good as the device. If used: per-person PINs, hashed, rate-limited with lockout,
     and only for a short list of entities.
- **Defence in depth on the HA side:** where the lock integration supports it, require a code
  in HA (`lock.unlock` with `code`), held by the backend and never sent to the frontend.
- **Audit:** every tier 1 and tier 2 action is logged as structured JSON with the principal,
  the device name, the action, the entity, the result, and for tier 2 how it was elevated.

### Frontend patches

All patches are listed in `PATCHES.md`:

- The "Not connected" takeover becomes the pairing screen (code plus QR, polling
  `/api/pair/status`).
- Handle `403 step-up`: redirect to login on the phone page, and on the frame show the "approve
  on your phone" or "use your phone" panel.
- The doorbell takeover's hold-to-unlock goes through the same tier 2 path.

## Alternatives considered

- **Keep the shared key.** It's simple and already built, but it has no revocation, no identity,
  and a rooted device can unlock the door. It's acceptable only while no tier 2 entities are
  exposed.
- **A per-device static key issued by the dashboard.** It's revocable per device and doesn't
  need Keycloak for the frame. But it's another credential registry next to Keycloak, it still
  needs a secret entered on the frame (through the URL), and it gives no approver audit. Pairing
  gives all of this with no secret on the device.
- **A Keycloak service account (client credentials) per frame.** This is a true machine
  identity, but the client secret has to live on the frame, which puts us back at an extractable
  static secret, now valid against Keycloak too.
- **mTLS client certificates.** Strong in principle, but the Android 6 WebView only presents a
  client certificate if the kiosk app implements `onReceivedClientCertRequest`, which upstream's
  APK doesn't. It would also need a private CA and Traefik TLS options. It could come back if the
  kiosk APK is ever forked.
- **oauth2-proxy with Traefik forward-auth.** No app code for phones, but it guards the whole
  hostname. The frame would need a bypass, and the obvious bypasses don't hold: a source-IP
  allowlist is unreliable behind SNAT, and a second hostname is reachable through the same
  Traefik IP. It also gives the app no identity for step-up decisions.
- **Keycloak CIBA for step-up on the frame.** This is the standard version of "approve on phone",
  but Keycloak's CIBA needs an external authentication channel provider. That's too much
  machinery for one household.

## Rollout

1. **Now (PRD 002 delivery 1):** shared key, unchanged.
2. **With the HA gateway delivery:** tiers are enforced from day one, with tier 2 **refused for
   everyone** (no step-up mechanism yet). Locks and garage doors are shown, but they can only be
   *locked* or *closed* from the dashboard.
3. **Keycloak login for phones,** with the shared key still accepted. Phones move to OIDC. Step-up
   (`aal2`) for phones turns tier 2 on for phones.
4. **Frame pairing.** The frame is paired once, its `?key=` is removed from the kiosk URL, then
   `HQ_KEY` is removed and the shared-key path is deleted.
5. **Step-up from the frame:** option 2 (approve on phone) or 3 (per-action QR), decided then.

## Open questions

- **Persistence for sessions.** Device sessions (and possibly phone sessions) must survive
  restarts. This is a small, encrypted, dashboard-owned state file (a few KB on a tiny PVC),
  which ties into the deferred notes and bins storage question. Could sessions live elsewhere
  instead, such as Keycloak user attributes? Probably not worth it.
- **Which step-up option for the frame**, 2 or 3. Option 2 depends on the HA companion app on
  every approver's phone. Option 3 depends on how slow a QR plus WebAuthn feels when someone is at
  the door.
- **Elevation scope:** one action per step-up, or a short window? The proposal is a 2-minute
  window on phones and one action on the frame.
- **ADB exposure:** can the router playbook also stop other IoT clients from reaching the frame's
  ADB port (client isolation on the IoT SSID)? That would reduce how exposed the frame's session
  is.
