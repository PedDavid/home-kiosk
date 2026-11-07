# Frame runbook: PF1007L as the Home HQ wall display

This is the one-time setup that turns a PF1007L (Allwinner A33, Android 6) into the Home HQ
frame, written so the frame can be rebuilt from scratch (PRD 002, user story 38). It covers the
network, the trust store, the kiosk app and the checks.

Upstream's device notes are the reference for anything below this level (rooting, upgrading the
WebView to 106, firmware quirks):

- `docs/allwinner-a33-pf1007l.md`
- `docs/allwinner-a33-mainline-uboot.md`

Both are in [sambartle/photo-frame-dashboard](https://github.com/sambartle/photo-frame-dashboard).
The kiosk app's own README is vendored at [`frontend/kiosk/README.md`](../frontend/kiosk/README.md).

## Before you start

- **WebView 106** is installed, and the shell is root (`adb shell id` shows `uid=0`). If not,
  follow upstream's PF1007L guide first.
- **USB ADB** works from your machine. Every step below uses `adb -d` (USB). The frame's wireless
  ADB stays off.
- **The dashboard is deployed** and works from the LAN: `https://home-hq.prdv.cloud/?key=…`
  shows the dashboard. See [`charts/home-hq/README.md`](../charts/home-hq/README.md).
- **You have the `HQ_KEY` value** from the Secret.

Set these once in your shell:

```bash
FRAME_URL='https://home-hq.prdv.cloud/?frame=kitchen&key=<HQ_KEY>'
```

`frame=kitchen` names this frame in check-ins and in the display controls.

## 1. Network (router playbook, infra repo)

The frame's Wi-Fi MAC is random on every boot (RTL8723CS), so nothing on the router can key on
it: no DHCP reservation, no per-MAC rule. The frame holds a fixed address of its own instead,
and the router's rules key on that.

1. **A static IP on the frame**, in the IoT subnet and outside the DHCP pool: `192.168.10.160`.
   In Android Settings → Wi-Fi, long-press the IoT network → Modify network → Advanced options →
   IP settings: Static. Use the router as gateway and Pi-hole as DNS. Check it with:

   ```bash
   adb -d shell ip -4 addr show wlan0
   ```

2. **The IoT→LAN allow rule** (`Allow-IoT-Home-HQ` in `router/playbook.yml`): source
   `192.168.10.160`, destination the dashboard's own LAN address (the chart's `lanService`, a
   MetalLB IP, `192.168.8.56`), TCP 443 only. Nothing else goes from IoT to LAN. If the frame
   ever loses its static IP, it falls back to DHCP and the dashboard stops loading. It never
   gains access.
3. **No internet.** The IoT zone is default-deny to WAN (infra RFC 020), so vendor firmware and
   apps can't phone home and the frame needs no rule of its own. The clock comes from the
   router's NTP if it serves one. Without NTP, check that the frame's clock is right, because
   TLS fails on a wrong date.
4. **DNS** goes through Pi-hole as for every IoT client, and `home-hq.prdv.cloud` must
   resolve to that LAN address.
5. **Recommended:** make sure other IoT clients can't reach the frame (client isolation on the
   IoT SSID, or a drop rule for traffic to `192.168.10.160`). The frame runs root ADB with no
   key check. Its wireless ADB is off by default, but isolation keeps it that way if something
   turns it on.

The address belongs to the dashboard alone: the app terminates TLS itself and nothing else listens
there. That's why it isn't Traefik's IP, which would let the frame reach every other ingress by
sending a different hostname.

The source-IP scope is defense in depth, not a hard wall: another device on the same Wi-Fi can
claim `192.168.10.160` (at the cost of an IP conflict). The dashboard's own key check is the
real boundary.

## 2. Trust store

The frame validates the dashboard's certificate, so the key is never sent over an untrusted
connection. The certificate is from Actalis: `home-hq.prdv.cloud` → Actalis DV TLS Server RSA CA
2025 → Actalis TLS Server RSA Root CA 2025 → Actalis Authentication Root CA. Android 6 ships that
last root, so there's nothing to install. Check it's there:

```bash
adb -d shell ls /system/etc/security/cacerts/5f47b495.0
```

And check the served chain from the LAN:

```bash
openssl s_client -connect home-hq.prdv.cloud:443 -servername home-hq.prdv.cloud -showcerts </dev/null | grep -E 's:|i:'
```

### Fallback: ISRG roots

Only if the dashboard ever moves to a Let's Encrypt certificate (chain: Let's Encrypt YR2 → ISRG
Root YR, cross-signed by ISRG Root X1). Android 6 trusts neither root, so both go into the system
CA store.

1. **See what is actually served**, from the LAN:

   ```bash
   openssl s_client -connect home-hq.prdv.cloud:443 -servername home-hq.prdv.cloud -showcerts </dev/null | grep -E 's:|i:'
   ```

   The last issuer tells you which root the frame has to know. Install both anyway.

2. **Download the roots** from <https://letsencrypt.org/certificates/>: ISRG Root X1 and ISRG
   Root YR, as PEM. Check each SHA-256 fingerprint against that page *and* a second source:

   ```bash
   openssl x509 -in isrg-root-x1.pem -noout -fingerprint -sha256
   openssl x509 -in isrg-root-yr.pem -noout -fingerprint -sha256
   ```

   ISRG Root X1 must be
   `96:BC:EC:06:26:49:76:F3:74:60:77:9A:CF:28:C5:A7:CF:E8:A3:C0:AA:E1:1A:8F:FC:EE:05:C0:BD:DF:08:C6`.

3. **Build files in Android's format.** The filename is the old-style subject hash plus `.0`.
   The content is the PEM block followed by the text dump, like the stock files:

   ```bash
   for pem in isrg-root-x1.pem isrg-root-yr.pem; do
     h=$(openssl x509 -in "$pem" -noout -subject_hash_old)
     { cat "$pem"; openssl x509 -in "$pem" -noout -text -fingerprint -sha1; } > "$h.0"
     echo "$pem -> $h.0"
   done
   ```

   ISRG Root X1 comes out as `6187b673.0`. Check that no file with the same name already exists:

   ```bash
   adb -d shell ls /system/etc/security/cacerts/ | grep -E '^(6187b673|<yr hash>)\.0$'
   ```

4. **Install them.** Remount `/system` read-write, copy, fix owner, mode and SELinux label, then
   remount read-only:

   ```bash
   adb -d push 6187b673.0 <yr hash>.0 /sdcard/
   ```

   ```bash
   adb -d shell 'mount -o rw,remount /system && for f in 6187b673.0 <yr hash>.0; do cp /sdcard/$f /system/etc/security/cacerts/$f && chown root:root /system/etc/security/cacerts/$f && chmod 644 /system/etc/security/cacerts/$f && restorecon /system/etc/security/cacerts/$f; rm /sdcard/$f; done; mount -o ro,remount /system'
   ```

   ```bash
   adb -d reboot
   ```

5. **Check.** In Settings → Security → Trusted credentials → System, "ISRG Root X1" and "ISRG
   Root YR" appear.

**Undo:** remount read-write, delete the two files, remount read-only, reboot.

## 3. Kiosk app

Upstream's kiosk APK is a full-screen WebView with a two-finger menu, screen-off modes and
an update notice after a deploy.

1. **Build it** with [`frontend/kiosk/build.sh`](../frontend/kiosk/build.sh) (JDK 17, Android
   build-tools 34; the script header lists the packages), then install it:

   ```bash
   frontend/kiosk/build.sh
   ```

   ```bash
   adb -d install -r frontend/kiosk/build/homehq.apk
   ```

2. **Make it the Home app.** Allwinner firmware filters boot broadcasts, so a kiosk only comes
   back after a reboot if Android opens it as Home:

   ```bash
   adb -d shell am start -n com.wesbos.homehq/.MainActivity --ez home true
   ```

3. **Save the dashboard URL.** The quotes matter: the URL has to reach the frame's shell as one
   argument.

   ```bash
   adb -d shell "am start -n com.wesbos.homehq/.MainActivity --es url '$FRAME_URL'"
   ```

   On each launch the dashboard swaps `?key=` for its long-lived cookie and drops it from the
   address bar. The saved URL keeps the key, so the frame re-authenticates after a wipe. This is
   the exposure the [auth RFC](rfc-device-auth.md) removes later.
4. **Screen-off mode** `backlight`. On the PF1007L the touchscreen powers down with the display,
   so `lock` can't be woken by a tap:

   ```bash
   adb -d shell am start -n com.wesbos.homehq/.MainActivity --es screenoff backlight
   ```

5. **Wireless ADB stays off.** Don't save an `adbfrom` subnet unless you need over-the-air
   installs. If you do, use your workstation's subnet, never `any`, and remember that the router
   only lets the frame reach the dashboard's LAN address anyway.

## 4. Checks

- [ ] **The dashboard loads** with dummy data, with no "Not connected" screen and no
      `net_error -202`. If you see the error, the trust store or the clock is wrong:
      `adb -d logcat | grep -i -E 'cert|trust|net_error'`.
- [ ] **The frame checks in.** Hold the clock to open the display controls; Status lists the
      frame named `kitchen` after about 5 minutes.
- [ ] **Reboot test.** Power-cycle the frame and touch nothing. The dashboard comes back on its
      own after Wi-Fi connects (the first load may fail and retry).
- [ ] **Screen-off test.** From the display controls choose Modes → Screen off. The panel goes
      fully black, and a tap brings it back.
- [ ] **Deploy pickup.** After the next image tag is deployed, the frame shows an **Update available — tap to refresh**
      notice within about 5 minutes (it compares `/build.json`). It chimes once per detected build,
      respecting mute, volume and quiet hours. Tap the notice to load the update; it never
      automatically reloads. The notice waits while the screen is off, photos are explicitly
      full screen, or a dialog is open.
- [ ] **Isolation.** The frame can't reach the internet, from the kiosk's two-finger menu →
      Android settings → any online check, or from `adb -d shell ping -c1 1.1.1.1`. From
      another IoT client, the frame's port 5555 is unreachable.

## Rebuilding from scratch

The steps don't depend on each other's leftovers. On a new or reset frame, run sections 2–4 in
order, after upstream's rooting and WebView steps, and set the static IP from section 1 first.
The router rules key on that IP, so they don't change when the frame does.
