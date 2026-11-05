# Home HQ kiosk navigation

Hold two fingers still on the dashboard for 1.5 seconds to open the native controls. Keep the
fingers at least an inch (about 2.5 cm) apart: on PF1007L frames the `gslX680` touchscreen reads
closer fingers as one touch.

- **Open launcher** opens another installed Home app, such as Rootless Launcher.
  If none is available, it opens Android Settings.
- **Android settings** opens the device's Settings app.
- **Wi-Fi settings** opens Android's Wi-Fi list directly.
- **Reload dashboard** reloads the current URL.
- **Back to dashboard** dismisses the controls.

## Optional single-tap Menu button

On panels where the two-finger gesture is unreliable, enable a native **Menu** button:

```sh
adb shell am start -n com.wesbos.homehq/.MainActivity --ez menu_button true
```

It opens the same controls with one tap. The preference survives later launches; the button
is hidden by default. Turn it off with the same command and `--ez menu_button false`.
The two-finger gesture remains available in either mode.

The button occupies 72×48 dp, with 12 dp margins at the bottom left, above both the page and
the offline screen. Leave that area clear in your dashboard when enabling it. It needs no
root or additional permissions. The button and physical single-tap interaction were tested
in a local adaptation on the [QCREA YYD-1156](../../docs/qcrea-yyd-1156.md); the saved opt-in is
the shared-kiosk version of that adaptation.

## Dashboard URL

The kiosk opens the hosted demo dashboard by default. To show your own, save its URL once:

```sh
adb shell am start -n com.wesbos.homehq/.MainActivity --es url https://your-dashboard/
```

`scripts/set-kiosk-url.sh <url>` from the repository root does the same, and prints the saved
URL when run without one.

Every later launch, including after a reboot, opens the saved URL. Links that other apps open
in the kiosk, such as a sign-in page, are shown but not saved.

## What the page can ask the kiosk (1.2, 1.3, 1.4)

Pages shown in the kiosk get `window.HomeHQKiosk` with two calls:

- `setScreenOff(true|false)`: screen off or back on, see [Screen sleep](#screen-sleep-13-131). A new
  page always starts at normal brightness.
- `info()`: JSON with the kiosk version, device model, Android version, Wi-Fi signal in dBm,
  `sleep` (whether the sleep right below is enabled), `screenOn`, and the wireless ADB state
  (`adbLeft` seconds, `adbFrom`, `adbIp`, `adbError`).
- `setAdb(true|false)`: wireless ADB on for 5 minutes, or off now; see
  [Wireless ADB](#wireless-adb-14).

When the screen comes on from a touch or the power button, the kiosk calls
`window.HomeHQScreenOn()` in the page, which the dashboard uses to leave "screen off".

## Screen sleep (1.3, 1.3.1)

Android won't let a plain app put the screen to sleep, and a window brightness of 0 only goes down
to the panel's minimum (8/255 on the C-2, 10/255 on the PF1007L), so the screen stays faintly lit.
1.3 can do what the power button does. Enable its sleep right once per frame, over ADB:

```sh
adb shell dpm set-active-admin com.wesbos.homehq/.ScreenAdmin
```

This registers the kiosk as a device administrator with a single right, force-lock (screen off and
lock; it can't wipe or change anything). Then `setScreenOff(true)` calls `lockNow()` and holds a
wake lock, so the CPU and Wi-Fi keep running while the panel is dark (alerts, check-ins and the
page's timers carry on). `setScreenOff(false)` wakes it, and a touch wakes it too. Without the
right, it falls back to dimming the backlight, as in 1.2.

Measured on the C-2 (2026-10-24): sleep with the key event gave `backlight(0)`, and the dashboard
still checked in 3 minutes later with the screen dark.

Not every frame can wake from this. On the **PF1007L** the touchscreen powers down with the display
(it sent no events at all while asleep, 2026-10-24), so a tap can't wake it. 1.3.1 adds a second way
for such frames, chosen once per frame and remembered:

```sh
adb shell am start -n com.wesbos.homehq/.MainActivity --es screenoff backlight   # or lock, dim, auto
```

- `lock`: the sleep above (the default when the admin right is enabled).
- `backlight`: the frame stays awake, and "screen off" writes 0 to the display driver's backlight
  level (`/sys/class/disp/disp/attr/lcdbl`, Allwinner) through `su`; the saved level goes back on
  wake. The panel is fully black and the touchscreen still works, so a tap wakes it. Android's auto-brightness
  would push the level back up whenever the room light changed (measured: the panel lit for ~9 s each
  time), so while dark it also switches `screen_brightness_mode` to manual (`settings put` via `su`),
  and back to auto on wake. It re-applies the 0 every 0.5 s as a backstop, checks the write took (if
  `su` refuses it dims instead), puts the level and auto-brightness back when the kiosk pauses or
  closes, and on the next start if it was killed while dark.
  Needs `su` available to apps (it was on both of our frames). It tries `su 0 sh -c …` first (the
  Android 6 `su` of the PF1007L rejects `-c`), then `su -c …`.
- `dim`: window brightness 0, which only reaches the panel's minimum.
- `auto`: forget the choice (lock if the admin right is on, else dim).

`info()` reports the mode as `screenoff` and whether the screen is on as `screenOn` (false while held
dark). The dashboard ignores a tap in the first 1.5 s after going off.

To undo the admin right: Settings → Security → Device admin apps → Home HQ → Deactivate (it can't be uninstalled
while it's an administrator, and `dpm remove-active-admin` refuses non-test apps).


The dashboard uses them to turn the screen off while everyone is away and to report each frame's
health in the display controls. Any page shown in the kiosk can call them, so they do nothing more
than this. Wi-Fi signal needs `ACCESS_WIFI_STATE`, a normal permission granted at install.

## Wireless ADB (1.4)

Installing an update is easier over Wi-Fi than finding the cable. Hold the clock on the dashboard (or
press **D**), find **Wireless ADB** in the display controls and tap **Turn on for 5 min**. It shows the
address to connect to, counts down, and switches itself off. It's the same as `adb tcpip 5555`:
`service.adb.tcp.port` set and `adbd` restarted through `su`, so a reboot closes it too. The timer is kept
by the kiosk and survives the restart that an install causes; from 1.4.2 a detached shell also closes it
if the app itself has died (a token check, so a newer window isn't cut short). The subnet must be /16 or
narrower, and the IPv6 block has to succeed as well as the IPv4 rules, or it stays closed.

**Be careful: these frames run root ADB with no key check (`ro.adb.secure=0`),** so while it's on, anyone
who can reach port 5555 has a root shell. That's why it's off by default, only started by someone at the
frame, limited to 5 minutes, and **it won't start at all until a subnet is saved** (1.4.1; 1.4 would open to
the whole network). Save the subnet your computer is on, once per frame over USB:

```sh
adb shell am start -n com.wesbos.homehq/.MainActivity --es adbfrom 192.168.1.0/24    # none removes it
```

With the subnet saved, the kiosk first adds firewall rules (an `iptables` chain `homehq_adb`: accept that
subnet on port 5555, drop the rest; IPv6 is blocked outright) and **leaves ADB closed if it can't make
them**. To open it to the whole Wi-Fi on purpose, save `any` instead of a subnet (`--es adbfrom any`); the
control then says so. `none` removes the saved value, after which it won't start. The subnet has to be able to
reach the frame (a router or firewall rule, and not a guest network that isolates its clients).

Then, with the frame's address from the control:

```sh
adb connect 192.168.40.23:5555    # and `adb disconnect` when you're done
```

Frames with a private deploy script can use it to do all of that (this repository has none, since
`frames/` is gitignored; [DEVLOG.md](../DEVLOG.md) describes the two we use: a USB one, and a Wi-Fi one
that checks the frame is reachable, connects, runs the USB script's steps and always disconnects).
The first install of 1.4 has to go over USB. Older kiosks don't show the control.

## A page bundled in the APK (no server)

A static page can ship inside the APK instead of being hosted. Build with `KIOSK_SITE` set to a
folder that has an `index.html` (from `dashboard/`):

```sh
KIOSK_SITE=/path/to/site ANDROID_HOME=/path/to/android-sdk bash kiosk/build.sh
adb install -r kiosk/build/homehq.apk
adb shell am start -n com.wesbos.homehq/.MainActivity --es url 'http://127.0.0.1:8765/'
```

On Windows, set `$env:KIOSK_SITE` before `.\build.ps1`.

Home HQ answers `http://127.0.0.1:8765/` from those files itself. Nothing listens on the port, no
other machine is needed, and the page loads without Wi-Fi. Every other URL, including the page's
own API calls, goes to the network as usual. ES modules work, which a `file://` page can't do, and
`localStorage` belongs to the `http://127.0.0.1:8765` origin. A query string
(`http://127.0.0.1:8765/?room=kitchen`) reaches the page unchanged; quote the URL in `adb shell`.
Don't put API keys or tokens in the bundled files or the saved URL: anyone with ADB on the frame
can read them, and so can any page shown at that origin. Keep them on a server
([building-for-the-frame.md](../../docs/building-for-the-frame.md)).
To change the page, rebuild and `adb install -r`. Without `KIOSK_SITE` the APK bundles nothing
and the address answers 404.

## Start at boot

To have the frame boot into the dashboard, make the kiosk its Home app. Installing the kiosk
does not offer it as a Home app, so nothing changes until you run:

```sh
adb shell am start -n com.wesbos.homehq/.MainActivity --ez home true
adb shell cmd package set-home-activity com.wesbos.homehq/.Home
```

Wait about 30 seconds before rebooting. Android saves the Home alias a few seconds later, and on
a PF1508 (Android 15, #167) a reboot 2 seconds after these commands turned the alias back off, so the
frame booted into the previous Home app. The same delayed save lost a `pm disable-user` on a
Pastigio FRAME156 (#139).

Android 6 frames (PF1007L, BIUFRAME) don't have `cmd package set-home-activity`. After the first
command, open the Home chooser instead, then tap **Home HQ** and **Always** on the frame. Keep
another launcher installed so **Open launcher** has somewhere to go.

```sh
adb shell am start -a android.intent.action.MAIN -c android.intent.category.HOME
adb shell 'grep -A2 "<preferred-activities" /data/system/users/0/package-restrictions.xml'   # check it stuck
```

The kiosk then opens the saved URL at boot, and the Home button returns to it. If the page
fails to load at boot because Wi-Fi isn't up yet, the kiosk keeps retrying. In the controls,
**Open launcher** skips the kiosk and opens another Home app. To undo, pick another Home app in
Settings → Apps → Default apps, then run the first command with `--ez home false`.

## Screen density

The dashboard is laid out for 1280×800 at density 160. On wider screens, such as 1920×1080 frames,
it keeps the 800 px height and widens to fill the screen. Most 1280×800 frames ship at 160, but a VESWWE
`DP101` shipped at 190 and the bottom of the dashboard was cut off
([veswwe-dp101.md](../../docs/veswwe-dp101.md)). Check with `adb shell wm density`. If it isn't 160:

```sh
adb shell wm density 160      # undo: adb shell wm density reset
```

Android's own menus get smaller too. The setting survives a reboot.

## Frames without Home or Back buttons

Some frame firmware omits Android's navigation bar. The two-finger gesture only
works inside Home HQ, so it cannot bring you back from another app's settings.

Enable **Home HQ navigation** in Android **Settings → Accessibility**. The
optional service adds floating **Back** and **Dashboard** buttons outside Home HQ:

- **Back** performs Android's normal Back action.
- **Dashboard** returns directly to Home HQ, keeping the current dashboard URL.
- Drag the dots to move the buttons. Their position is saved.

The buttons hide while Home HQ is active and return when you leave it. Android
keeps the service enabled across reboots. It requests no accessibility events or
screen-content access and does not require root. Disable **Home HQ navigation**
in Accessibility settings to remove the buttons.

On a PF1007L (`20260519` firmware), the vendor `BackgroundManagerService` dropped the
service from the enabled list after every `adb install -r` or force stop of Home HQ, so the
buttons silently disappeared. To have Home HQ put the service back each time it starts, grant
it this permission once:

```sh
adb shell pm grant com.wesbos.homehq android.permission.WRITE_SECURE_SETTINGS
```

With the grant, Home HQ also rebinds the service if it is still listed but not running, which is
what Android 6 leaves behind when Home HQ's process is killed. If the buttons keep disappearing
whenever you leave Home HQ, the firmware may be killing every app you switch away from; see
[the Denver PFF-1015 notes](../../docs/denver-pff-1015.md#low-ram-mode-kills-every-app-you-leave).

The grant survives updates. To turn the buttons off for good, revoke it first
(`pm revoke` with the same arguments), then disable the service in Accessibility settings.

January `20260113` and May `20260529` firmware were also observed keeping a navigation service in
the selected list but resetting `accessibility_enabled` to `0` after boot. On `20260529` (#234)
the buttons came back on every boot once Home HQ was the Home app (see [Start at boot](#start-at-boot)). With the same optional grant,
Home HQ repairs either state when it starts, without rewriting an already selected list.
This reset was measured with a separate navigation app; the Home HQ change has host regression
coverage but has not been installed on that unit. See the
[January report](../../docs/allwinner-a33-pf1007l.md#9-january-unit-remote-management-and-background-services).

Keep an ADB connection available while enabling the service on a frame with no
other way back. If needed, return to the kiosk with:

```sh
adb shell am start -n com.wesbos.homehq/.MainActivity
```

This service supplies navigation controls only. To start the dashboard at boot,
make Home HQ the default Home app (see [Start at boot](#start-at-boot)).

To build your own dashboard for a PF1007L frame, see
[building-for-the-frame.md](../../docs/building-for-the-frame.md): what WebView 106 supports,
what to avoid, and how to test on the frame.

## When the dashboard can't load

If the dashboard fails to load, Home HQ covers WebView's error page with its own
screen. The error page would print the full URL, including any token in it.

- **No Wi-Fi:** "Connect to Wi-Fi?" with an **Open Wi-Fi settings** button.
- **Wi-Fi is up but the dashboard is unreachable:** "Can't reach the dashboard".
- **Wi-Fi has a sign-in page** (a captive portal, as on school, hotel and guest networks): the
  kiosk checks `http://connectivitycheck.gstatic.com/generate_204` the way Android does. A redirect
  or a page instead of the empty 204 means a portal, and the kiosk opens its page with a bar above
  it, so the terms can be accepted on the frame. It checks again every 5 seconds and returns to the
  dashboard once the frame is online. After 10 minutes it goes back to retrying the dashboard. **Cancel** closes the page; the offline screen then offers **Sign in to Wi-Fi**
  until the frame joins a network again. Frames with SystemUI disabled never see Android's own
  "Sign in to network" notification, so this is the only way to reach the page.

- **Certificate error:** never bypassed. If the error is about the date, the screen says
  "Setting the clock…". Frames with no clock battery start with an old date after a power cut
  (a Runxion SSA ZN-DP1002 came up on May 14, 2021), so every HTTPS certificate fails until
  network time arrives, about 20 seconds after Wi-Fi joins.

Home HQ keeps retrying every 5 seconds, and retries at once when Wi-Fi connects or the clock
changes. The screen disappears once a load succeeds. Wi-Fi settings open only when tapped,
because Wi-Fi is often still connecting for the first minute after boot.

On frames without a navigation bar (most PF1007L units), set up the navigation service in
[Frames without Home or Back buttons](#frames-without-home-or-back-buttons) first. Otherwise
**Open Wi-Fi settings** leaves no way back to the dashboard.

## Time zone

After changing the frame's time zone over ADB (`setprop persist.sys.timezone <zone>` and
`service call alarm 3 s16 <zone>` on Android 6; the transaction number differs on other versions), restart the kiosk. Until then the page keeps the old zone,
while `date` on the frame already shows the new one:

```sh
adb shell am force-stop com.wesbos.homehq
adb shell am start -n com.wesbos.homehq/.MainActivity
```

Seen on an Android 6 Runxion (WebView 107). Changing the zone in Android Settings has not been
compared.

## Build and install

Host-only navigation regression checks need Python 3 and a JDK on `PATH`:

```sh
python3 kiosk/test-navigation.py
```

Run from `dashboard/`. The test compiles the actual repair method with settings and
permission fakes; it does not prove service binding or reboot behavior on Android.

Use JDK 17, Android build-tools 34.0.0, and platform 34. On Ubuntu 24.04, the apt packages listed
at the top of `build.sh` install all three, with `ANDROID_HOME=/usr/lib/android-sdk`. From `dashboard/`:

```sh
ANDROID_HOME=/path/to/android-sdk bash kiosk/build.sh
adb install -r kiosk/build/homehq.apk
```

The kiosk loads the existing hosted dashboard over the frame's Wi-Fi. A USB
connection is only needed for installation or diagnostics, not normal use.

The kiosk targets Android 9 (SDK 28); the minimum is still Android 6 (SDK 23). Android 15
refuses to install targets below 24 (`INSTALL_FAILED_DEPRECATED_SDK_VERSION`), and Android 14
and later show a "built for an older version of Android" dialog on the first launch for targets
below 28.
