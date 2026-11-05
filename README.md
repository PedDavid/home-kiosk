# Home HQ dashboard

The family dashboard from the video, wired to real data. It runs as one Cloudflare Worker on **your
own** Cloudflare account (the free plan is enough). The Worker serves the page and a small API
behind it:

| On screen | Comes from |
| --- | --- |
| Calendar, Today lanes, "Next up" | Google Calendar secret ICS links, plus events added on the frame (also written to Google with a service account, optional) |
| Leave-by times | `Travel:` in the event, or a live drive time from the Google Routes API (optional) |
| Weather | [Open-Meteo](https://open-meteo.com) for your latitude/longitude (no API key) |
| Weather warnings | [Met Office](https://weather.metoffice.gov.uk/guides/rss) regional feed, kept to your areas (UK only, no key) |
| Bin day | Your council's collection rotation, written into `settings.json` |
| Shopping list, notes | Cloudflare D1, shared by every screen and phone that opens the dashboard |
| Photos (slideshow, home tile, ambient mode) | Uploaded from a phone at `/upload`; stored in Cloudflare R2 |
| Home and Media tabs, presence, energy, doorbell/leak/smoke/garage alerts, screen off when away | Home Assistant REST API (optional) |

The kiosk app in [`kiosk/`](kiosk) shows it full screen on the frame. [DEVLOG.md](DEVLOG.md) has the
technical history: architecture, design decisions, version by version, and the gotchas.

## One-time setup

You need Node 20+, a free [Cloudflare account](https://dash.cloudflare.com/sign-up), and (for the
Home tab) Home Assistant reachable over HTTPS.

```bash
cd dashboard
npm install
npx wrangler login
```

1. **Database and photo bucket.** Create both on your account:

   ```bash
   npx wrangler d1 create home-hq
   npx wrangler r2 bucket create home-hq-photos
   ```

   Paste the `database_id` that the first command prints into `wrangler.jsonc`, replacing
   `REPLACE_WITH_YOUR_D1_ID` (and your domain in `routes`). If you commit your own copy of the repo,
   keep the real values in `wrangler.local.jsonc` instead (gitignored; `npm run deploy` and
   `db:migrate` use it when it exists) so they stay out of it. Then create the tables:

   ```bash
   npm run db:migrate
   ```

2. **Your settings and secrets.** Copy both examples and edit them. Both files are gitignored.

   ```bash
   cp settings.example.json settings.json
   cp secrets.example.json secrets.json
   ```

   `settings.json` is what the dashboard shows and does; it's built into the deployed code.
   `secrets.json` holds what grants access, and is uploaded as encrypted Cloudflare secrets instead:

   - `people` (settings): names and colours. Add `"ha": "person.x"` to show whether they're home.
   - `home` (settings): delete this section if you don't use Home Assistant; the Home tab then
     disappears.
   - `DASHBOARD_KEY` (secrets): a long random key, letters and digits only (`openssl rand -hex 24`) so
     it survives `adb shell`. Every screen and phone needs it once.
   - `HA_TOKEN` (secrets): only with Home Assistant; see [Connect it](#connect-it).
   - `CALENDARS` (secrets): in Google Calendar, open **Settings → (the calendar) → Integrate calendar →
     Secret address in iCal format**. Give each link the people it belongs to, or `"all"`. If the
     same invite appears on two calendars, it shows once with both people. To get a "Leave by"
     reminder, add `Travel: 15` (minutes) anywhere in an event's description. Anyone with one of
     these links can read that calendar. If one leaks, **Reset** it on the same Google page and
     upload the new one.

3. **Upload the secrets.**

   ```bash
   npm run secrets
   ```

   It checks `secrets.json` (valid JSON, a sensible key and token, calendar people that exist in
   `settings.json`), refuses if git could commit the file, makes it readable by you only, and
   uploads every secret that has a value in one go. Blank ones are skipped, so they never replace
   what's already on Cloudflare. Run it again after any change; no redeploy is needed.

   Cloudflare never shows a secret again, even to you, so `secrets.json` is your only readable copy.
   Keep it out of synced folders (Dropbox, Google Drive and so on) and back it up in a password
   manager.

4. **Deploy.**

   ```bash
   npm run deploy
   ```

   Wrangler prints your address: your own domain from `routes` in `wrangler.jsonc` (a domain on the
   same Cloudflare account; Cloudflare adds the DNS record and certificate), or
   `https://<name>.<you>.workers.dev` without one. Open it once as
   `https://<your address>/?key=<DASHBOARD_KEY>`. The key moves into a cookie and
   disappears from the address bar.

5. **Point the frame at it.** From the repository root, with the frame on ADB:

   ```bash
   scripts/set-kiosk-url.sh 'https://<your address>/?key=<DASHBOARD_KEY>'
   ```

6. **Add photos.** On a phone, open `https://<your address>/upload?key=<DASHBOARD_KEY>`,
   pick who's adding them, and choose photos. They're shrunk and rotated on the phone (1920 px and
   960 px copies) and dated from EXIF, so "On this day" works.

After editing `settings.json`, run `npm run deploy` again. Lists, ticks and photos live in D1 and R2
and aren't touched by a deploy.

## Using it

The frame is a touch screen laid out for 1280×800. The left rail switches tabs: **Today**,
**Calendar**, **Shopping**, **Photos**, **Home** and **Media** (the last two only with Home
Assistant), plus **Note**. With a keyboard, keys **1**–**6** pick the tabs, **A** rests
the frame now, **D** opens the display controls and **Esc** closes things.

**Screens by themselves.** After a minute untouched the frame goes back to Today. After two it
(20 seconds in the night hours, so a look at the time doesn't keep the screen up) rests, and the clock chooses how: the photo screen by day, a dim clock between **Dim** and
**Bright** (10 PM to 6 AM unless you change them), or the screen off between **Sleep** and **Wake**
if you've switched that on. A tap wakes it to Today, and from then on it's just the dashboard until
it next goes quiet. Between Dim and Bright the whole dashboard is dimmed and amber, tapped
awake or not (alerts are never dimmed), so a tap in the night doesn't flash a bright screen. The
same happens at the boundaries: a frame resting on photos when Dim arrives becomes the clock, and
so on. One that someone is using carries on until it next rests.
With Home Assistant and `display.awayOffMinutes`, the screen goes off while everyone is out (see
[Home Assistant](#home-assistant)).

### Today

- **Next up** is the next event. With a leave-by time (see [Calendars](#calendars)) it counts down
  "Leave for … min to leave" in the last hour, turns green with **Leave now** (and a chime), then
  "Running late" while anyone on the event is still home. Tap the event for the details, including
  where the drive time came from.
- **Weather** shows today and tomorrow; a Met Office warning for your areas replaces the forecast
  line until it ends. Tap it for the details.
- **Bins** shows the next collection. From 4 PM the day before, the card turns dark and asks for the
  bins to go out, with a banner at 6 PM and 8:30 PM, until someone taps **They're out**.
- **Home card** (Home Assistant): the energy line (kWh so far today, or the current draw when it's
  high; tap for the Energy panel) and the **all clear** line (the first thing that needs a look,
  such as a door unlocked or a sensor battery low; tap for the list).
- **Notes** float bottom left (see below). Who's home is top right, under the Home HQ wordmark.

### Calendar

**Today**, **3 Day**, **Week** and **Month** views; the day grids cover the whole 24 hours and
open at 6 AM. Tap an event for its
details. **+** adds an event: pick who it's for (a person or **Everyone**), type a title, then tap
the date or time chip to pick any date and start time, and choose how long it lasts. It appears at once, and goes
to Google Calendar too when the service account is set up. Events added on the frame show
**Remove** in their details, which removes them from Google as well; the toast offers **Undo**.
Events from Google are changed in the Google Calendar app.

### Shopping

Tap a **Usual** to add it, or type any item (known items go to their aisle, others under "Other").
The store buttons filter the list. Tap an item to tick it off: it moves to **Got it** at the bottom,
where it can be put back, and **Clear** empties the ticked ones (with Undo). Phones see the same list within 30 s.

### Photos

The Photos tab is a slideshow of everything uploaded, with "On this day" photos from earlier years.
The heart marks a favourite, which comes up more often. Adding and deleting photos is done on a
phone; see [Adding photos](#adding-photos).

### Notes

The **Note** button in the frame's left menu, or **Notes** on the phone page, pins a short
note (140 characters) on every frame: from someone, for everyone or one person, kept until someone
taps **Done** or until tonight, tomorrow evening or a date. On Today the newest three float
bottom-left, tinted in the writer's colour; other tabs show a "2 notes" pill; the photo screen shows
the newest; night mode and screen-off hide them. A note for one person also goes to their phone when
`settings.json` gives that person a `"notify": "notify.mobile_app_…"`. Frames chime when a new one
arrives (quiet at night). Only the frame a note was written on offers **Edit**.

### Media

One card per media player Home Assistant knows, playing first: play/pause, previous, next, volume
and mute, as each player supports. A player with nothing loaded can't be started from here (start
it from its own app first); its buttons are greyed out.

### The phone page

Phones open the same address and get a phone page with **Shopping**, **Notes** and **Photos**
(upload and delete), and a link to the wall view. It warns when a frame has gone quiet. On Android,
install it as an app to share photos straight from the gallery (see [Adding photos](#adding-photos)).

### Display controls

Press and hold the clock in the left rail (or press **D**) to open them. They show:

- **Status:** calendar events and photos loaded, weather, and Home Assistant ("HA connected", or
  its error), with the dashboard version underneath.
- **Night mode:** four times kept on that frame, so each frame can differ (the bedroom from the
  kitchen). **Dim** and **Bright** are when the photo screen turns into a dim clock and when it goes
  back (10 PM and 6 AM until set); chimes stay quiet in between. **Sleep** and **Wake** are when an
  untouched frame goes fully dark and comes back (1 AM and 5 AM suggested; kiosk 1.3 with its
  sleep right enabled puts the screen fully to sleep, older ones only dim it to the panel's minimum); they only apply when the
  button beside them says **On**. A tap or any alert wakes a sleeping frame, and it goes dark again
  after two minutes untouched. A window may cross midnight. Clearing the frame's storage resets the times.
- **Timeouts:** three times in seconds, kept on that frame: **Today** (back to Today after this long
  untouched, 60), **Rest** (then the photo screen, dim clock or screen off, 120) and **Peek** (the
  Rest time instead, during the night hours, 20). Anything outside 5–3600 snaps back.
- **Modes:** a dropdown and **Show**. **Ambient** rests the frame now instead of waiting for the
  timeout; the clock still chooses the screen (photos, dim clock or off) and keeps switching at
  Dim, Bright, Sleep and Wake as normal. **Screen off** turns the screen as dark as it goes and keeps
  it that way, whatever the time, until someone taps it. A tap checks the clock and shows Today
  dimmed or normal as the hour requires. The circular-arrow button beside it refreshes all the data
  at once.
- **Test alerts:** a dropdown that shows each alert without touching the house: doorbell, water
  leak, smoke, garage open, weather warning, bins out tonight, a frame notification to the phones
  (this one really sends), the Leave now chime and a sound check. The bins and weather tests clear
  themselves after two minutes and save nothing.

**Sound.** Frames chime for the doorbell, "Leave now" and the bins reminder, and sound a repeating
alarm with the smoke, carbon monoxide and water leak takeovers until it's dismissed. The sounds are
generated in the page, so there are no audio files. Chimes stay quiet in the night hours, in night
mode and while everyone is away; alarms always sound, in addition to the real alarms. **Sound on
this frame** in the display controls mutes the chimes and sets a volume, both remembered per frame,
on top of Android's media volume (the frame's volume buttons, where it has them). Test alerts plays
each sound, and **Sound check** plays a tone.

**Frames** in the display controls lists every wall display that has checked in (each does every
5 minutes): its dashboard and kiosk versions, Wi-Fi signal (kiosk 1.2) and when it was last seen.
One quiet for 15 minutes turns red there and shows a warning on the phone page. Only the kiosk
app (1.2 or later) checks in, so a phone or laptop showing the wall view never appears in the list.

**Naming a frame.** A frame is listed under its device model ("C-2") until you name it. Put the
name in the address the kiosk starts from, after the key:

```bash
scripts/set-kiosk-url.sh 'https://<your address>/?key=<DASHBOARD_KEY>&frame=Kitchen'
```

The kiosk opens that address at every start, so the name comes back even if the frame's storage
is cleared. A name has letters, digits and spaces, up to 40 characters; write a space as `%20`
(`&frame=Living%20room`). Renaming updates the frame's entry at its next check-in. After a move to
a new address a frame checks in as new (its id is stored per address); its old entry, same name and
quiet for 15 minutes, is removed then. Any other browser or screen can be listed the same way:
open its address once with `?frame=<name>`. Entries silent for 30 days drop off by themselves.

## Running it day to day

Run every `npx wrangler` and `npm run` command from the `dashboard` folder. From anywhere else
wrangler can't find `wrangler.jsonc` and stops with "Required Worker name missing".

| To change… | Edit | Then | Shows up |
| --- | --- | --- | --- |
| The dashboard code | the files in `dashboard/` | `npm run deploy` | frames reload themselves within ~6 min |
| People, stores, shopping catalog, weather location, bins, Home Assistant entities | `settings.json` | `npm run deploy` | frames reload themselves within ~6 min |
| Calendars, Google keys, alert token | `secrets.json` | `npm run secrets` | calendars within ~15 min, the rest at once; no deploy |
| Home Assistant token | a new one in Home Assistant, into `secrets.json` | `npm run secrets` | immediately |
| The dashboard key | `secrets.json` | `npm run secrets`, then re-key every device | immediately |
| Shopping list, notes, photos | on a screen or phone | nothing | within 30 s on other screens |

### Update the dashboard

```bash
npm run deploy                 # builds and uploads; takes about 20 s
```

Frames check for a new version every 5 minutes and reload themselves once nobody has touched the
screen for a minute; the version under **Status** in the display controls shows which one is
running. To pick it up at once, open the kiosk menu (hold two fingers still for 1.5 s) and tap
**Reload dashboard**. Phones get it the next time the page is opened.

The version number is `version` in `package.json`; bump it with each release so the frames show
which one they run, and note the change in [DEVLOG.md](DEVLOG.md).

If an update adds a file in `migrations/`, create the new tables **before** deploying, and check
they're there, or the new code fails until they are:

```bash
npm run db:migrate
npx wrangler d1 migrations list home-hq --remote   # should say no migrations to apply
```

To undo a bad deploy:

```bash
npx wrangler versions list     # find the previous version id
npx wrangler rollback <version-id>
```

### Family and shopping

All in `settings.json`, then `npm run deploy`:

- **People:** `id`, `name`, `color`. Add `"ha": "person.x"` for the home/away display. Changing
  someone's `id` detaches their calendar mapping.
- **Shopping:**
  - `stores` sets the filter buttons. New items go to the first store, or to the one being viewed.
  - `categories` sets the aisle order, and `storeOrder` can reorder the aisles per store.
  - `catalog` maps item names to aisles; unknown items go under "Other".
  - `usuals` are the quick-add tiles on the frame.
  - If you remove a store, items already listed under it still show under **All**.
- **Bins:** `types` names each bin with a `color` (and a `lid` colour when it differs); `every` gives each bin's rotation, as one
  collection date (`from`) and how many `weeks` apart. Copy them from your council's calendar.
  `moved` lists bank-holiday changes (`"usual date": "actual date"`, or `null` when skipped).
  The home screen shows the next collection; from 4 PM the day before it asks for the bins to go
  out, with a banner at 6 PM and 8:30 PM until someone taps **They're out**.
- **Weather warnings (UK):** `warnings.region` is a Met Office region code (`yh` Yorkshire &
  Humber, `se`, `nw`, `wl`, `os` …; the codes are on the Met Office RSS page) and `areas` lists the
  council areas you care about. A warning naming one of them replaces the forecast line until it
  ends; tap it for the details.

### Calendars

Each entry in `CALENDARS` in `secrets.json` is a Google Calendar secret iCal link and the people it
belongs to: `["sam"]`, `["laura", "sam"]` or `"all"`. After any change, upload the secrets again:

```bash
npm run secrets
```

The Worker re-reads calendars at most every 10 minutes and the frame asks every 5, so allow up to
about 15 minutes. To refresh straight away:

```bash
npx wrangler d1 execute home-hq --remote --command "DELETE FROM cache WHERE key = 'calendar'"
```

**Leave-by times.** Lines in an event's description set when to leave:

- `Travel: 25`: minutes door to door. Used as written when there's no live drive time.
- `Walk: 5`: minutes from parking to the door, added to the live drive time. Optional.

With a `GOOGLE_MAPS_KEY` in `secrets.json`, events that have a **location** and start in the next 4
hours get a live, traffic-aware drive time from Google instead of `Travel:`. It starts from wherever
Home Assistant places the event's people (their phone's position, from the companion app), or from
home (`zone.home`) for anyone it can't place; with several people the longest drive counts. Each
event is looked up at most every 10 minutes. If Google can't find the place, has no route, or the
drive is over 3 hours, `Travel:` is used. The event sheet says which was used. A lookup sends the
person's position and the event's location to Google.

To get the key, once:

1. In the [Google Cloud console](https://console.cloud.google.com), create a project (for example
   "Home HQ") and attach a billing account; Google requires one even within the free allowance.
2. **APIs & Services → Library**: enable the **Routes API**.
3. **APIs & Services → Credentials → Create credentials → API key**, then edit it: under **API
   restrictions** choose **Restrict key** and tick only the Routes API. Leave application
   restrictions off: the Worker calls from Cloudflare's servers.
4. The dashboard asks Google at most 300 times a day (then uses `Travel:` until midnight), so a fault
   can't run up a bill. On a paid account you can also cap it at Google: **Google Maps Platform →
   Quotas → Routes API → "Directions - ComputeRoutes per request quota per day"**. Free trial
   accounts can't edit quotas, but can't be charged either: the API just stops when the trial ends.
5. Put the key in `secrets.json` as `GOOGLE_MAPS_KEY` and run `npm run secrets`.

A typical family calendar needs about a thousand lookups a month, inside the monthly free allowance
for the Routes API at the time of writing; check Google's current pricing.

Add and change events in the Google Calendar app as usual; the frame follows within the same window.
To remove a calendar, delete its line and upload again. If a link may have leaked, also press **Reset** next to
it in Google Calendar's settings, which kills the old link, and put the new one in.

**Events added on the frame go to Google Calendar too** (optional). The Worker writes them with a
Google **service account**: a separate robot account in your Google Cloud project, signed in with
its own key through Google's documented server-to-server flow, that can only touch the calendars
you share with it. An event for everyone goes to the `"all"` calendar in `CALENDARS`, one for a
single person to that person's calendar (or the `"all"` one if they have none). The frame shows it
at once; when Google's iCal feed carries it, the feed's copy takes over. **Remove** or **Undo** on
the frame deletes it from Google as well. If Google refuses (calendar not shared, no key, offline),
the event stays on the frame and a message says so. A service account can't send invitations.

To set it up, once:

1. In the [Google Cloud console](https://console.cloud.google.com), in the same project as the Maps
   key (or a new one): **APIs & Services → Library**, enable the **Google Calendar API**.
2. **IAM & Admin → Service Accounts → Create service account**. Name it, for example `homehq`;
   skip the optional roles and access steps (it needs no project permissions). Open it, then
   **Keys → Add key → Create new key → JSON**. A `.json` file downloads.
   If Google says **"Service account key creation is disabled"**, your Google Cloud account belongs
   to an organisation (your own domain) with the secure-by-default policy
   `iam.disableServiceAccountKeyCreation`. As the organisation's admin: in the organisation's
   **IAM**, give yourself **Organisation Policy Administrator**; then in this project, **IAM & Admin
   → Organisation policies → Disable service account key creation → Manage policy → Override
   parent's policy**, enforcement **Off**, and create the key. Afterwards set it back to **Inherit
   parent's policy**: the key keeps working, and no new ones can be made. Without admin rights,
   make the project under a personal Google account instead, which has no such policy.
3. Paste that file's whole contents into `secrets.json` as the value of `GOOGLE_SERVICE_ACCOUNT`
   (the object, not a string), delete the downloaded file, and run `npm run secrets`. It prints the
   service account's email address.
4. In [Google Calendar](https://calendar.google.com), for **each calendar in `CALENDARS`**: the
   calendar's ⋮ menu → **Settings and sharing → Shared with → Add people and groups**, paste the
   service account's email, choose **Make changes to events**, and **Send**.
5. Test: add an event with **+** on the frame; it appears in the Google Calendar app within seconds.

### Updating a frame's kiosk app over Wi-Fi

The kiosk app (not the dashboard, which a deploy updates by itself) can be updated without the USB
cable: in the display controls, **Wireless ADB**, **Turn on for 5 min**, then `adb connect
<address>:5555` from a computer allowed to reach it. It switches itself off after 5 minutes, and can be
limited to your computer's subnet (it won't start until one is saved). These frames run root ADB with no key check, so read the
[kiosk guide](kiosk/README.md#wireless-adb-14) before using it.

### Connecting devices

- **A phone or computer:** open `https://<your dashboard>/?key=<DASHBOARD_KEY>` once. Phones get
  the phone page (shopping, photos); bigger screens get the wall dashboard. Open the address
  with `?wall` to see the wall dashboard on a phone, and with `?phone` to switch back.
- **A frame:** `scripts/set-kiosk-url.sh 'https://<your dashboard>/?key=<DASHBOARD_KEY>'` from the
  repository root, with the frame on ADB.
- **Change the key**, for example if a link with it was shared:
  put a new `openssl rand -hex 24` in `secrets.json` as `DASHBOARD_KEY` and run `npm run secrets`.
  Every device then shows "Not connected" until it has opened the address with the new key once;
  re-point each frame with `scripts/set-kiosk-url.sh` (keep its `&frame=` name).

### Adding photos

On a phone, open `/upload` (or **Photos** on the phone page). Pick **Added by**, an optional place,
and the photos. They're resized on the phone, and the location and other hidden metadata are
removed; only the date taken is kept. The same page lists every photo with a delete button. On the
frame, the heart on the Photos tab marks favourites, which come up more often.

**Share from the gallery (Android):** open the phone page in Chrome and choose **Install app** (or
**Add to Home screen**) from the menu, once per phone. **Home HQ** then appears when you tap
**Share** on photos in the gallery or Google Photos. They open on the upload page, ready to go:
add a place if you like and tap **Upload**. iPhones can't add web apps to the share menu, so they
keep using `/upload`.

### Moving to a new address

Renaming the Worker or changing its domain gives the dashboard a new address. The data stays (D1
and R2 are attached by id), but secrets belong to each Worker, and every screen and phone signs in
again because the cookie belongs to the old address. To move without visiting each one:

1. Change `name` and/or `routes` in `wrangler.jsonc`, `npm run deploy`, then `npm run secrets`.
2. Set `NEW_ORIGIN` and the old name in `tools/moved/wrangler.jsonc`, then
   `npx wrangler deploy -c tools/moved/wrangler.jsonc`. The old address now redirects every page to
   the new one, adding the key for anyone who already had it (the old cookie or `?key=`), and pages
   left open reload into it within a few minutes.
3. Point each frame at the new address with `scripts/set-kiosk-url.sh` (keep its `&frame=` name):
   the kiosk keeps starting from the address it was given, so until then it goes through the
   redirect. Reinstall the phone app from the new address (the share menu entry belongs to the
   old one).
4. With pushed alerts, change the address in Home Assistant's `secrets.yaml` line
   (`npm run ha-alerts -- --with-token` prints the new one) and restart Home Assistant. Until
   then its pushes go to the old address, and alerts arrive only by polling.
5. When nothing uses the old address any more, delete its Worker in the Cloudflare dashboard.

The Google service account and Maps key don't depend on the address.

### Backups

What isn't in git, and where its copy should live:

| What | Where it lives | Back it up by |
| --- | --- | --- |
| `secrets.json` | your machine only (Cloudflare never shows secrets again) | a copy in a password manager, after every change |
| `settings.json` | your machine only (gitignored); bundled into the deployed Worker | a copy somewhere safe (it holds entity ids and the bin rotation, nothing secret) |
| Shopping list, notes, frame-added events, photo list, bins "they're out" | D1 | `npx wrangler d1 export` (below) |
| Photos | R2 | nothing: they're copies of ones on your phone |
| Frame setup (private notes, build and deploy scripts) | `frames/` at the repository root, gitignored | a copy alongside `settings.json` |

```bash
npx wrangler d1 export home-hq --remote --output home-hq-$(date +%F).sql
```

D1 also keeps point-in-time history by itself (Time Travel: 7 days on the free plan, 30 on paid), which is the quickest way
back from a mistake:

```bash
npx wrangler d1 time-travel info home-hq                       # the bookmark for now
npx wrangler d1 time-travel restore home-hq --timestamp=2026-10-20T18:00:00Z
```

A restore replaces the whole database, so export first. The `chore_log` table holds chore ticks
from before chores were removed; nothing reads it.

To rebuild from nothing: create D1 and R2 as in [One-time setup](#one-time-setup), import the export
with `npx wrangler d1 execute home-hq --remote --file home-hq-<date>.sql` instead of
`npm run db:migrate`, put back `settings.json` and `secrets.json`, then `npm run secrets` and
`npm run deploy`. Photos would need uploading again.

### When something looks wrong

- **The frame or a phone says "Not connected":** it doesn't have the current key. Open the address
  with `?key=` again.
- **Taps do nothing on the frame:** reload it from the kiosk menu (two-finger hold). That fault was
  fixed in the kiosk app, so if it still happens, rebuild the kiosk from current code.
- **See what's connected:** on the frame, press and hold the clock in the sidebar (or press **D** on a
  keyboard). **Status** lists the number of calendar events, the number of photos, and the state of
  weather and Home Assistant, including Home Assistant's error message.
- **The Worker's live log:** `npx wrangler tail` shows each request and any error, such as a
  calendar link that stopped working or `exceeded CPU`.
- **Check the secrets are set:** `npx wrangler secret list` shows their names, never their values.

## Home Assistant

The Worker runs in Cloudflare's network, not your house, so it needs an HTTPS address for Home
Assistant: [Nabu Casa](https://www.nabucasa.com) remote access or a Cloudflare Tunnel. Only
entities listed in `settings.json` can be read or controlled through the dashboard.

### Connect it

1. **Address.** Use the external address you already open Home Assistant with away from home, for
   example `https://ha.example.com` from a Cloudflare Tunnel or `https://xxxx.ui.nabu.casa`. If that
   address sits behind Cloudflare Access (a login page), the Worker can't get past it. Add an Access
   policy that lets `/api/` through for a service token, or bypass Access for `/api/`, which still
   leaves Home Assistant's own token check in place.
2. **A user just for the dashboard.** In Home Assistant, open **Settings → People → Users → Add user**,
   for example "Frame". Leave **Administrator** off; it can still read states and switch lights,
   locks and scenes. Log in as that user, then open **Profile → Security → Long-lived access tokens →
   Create token**.
3. **The token**: put it in `secrets.json` as `HA_TOKEN`, then from the `dashboard` folder:

   ```bash
   npm run secrets
   ```

4. **The `home` section** in `settings.json`:
   - `url`: the address from step 1.
   - `rooms`: each with its `devices`, as `{ "entity": "light.kitchen", "name": "Kitchen" }`.
   - optionally `climate`, `cameras`, `scenes`, `players` and `alerts`. See `settings.example.json` and the
     table below.

   Add `"ha": "person.sam"` to each person who has a `person.` entity. Find entity ids in
   **Settings → Devices & services → Entities**. Then deploy:

   ```bash
   npm run deploy
   ```

5. **Check it.** The Home tab appears on the frame. **Status** in the display controls (hold the clock)
   says "HA connected", or "HA offline" with its error: a wrong address, a token rejected
   (`HTTP 401`), or the address unreachable from the internet.

To add or remove devices later, edit `home` and deploy again. If the token is lost or leaked, delete
it under that user's **Profile → Security** and put the new one in `secrets.json` and run `npm run secrets`.
To turn Home Assistant off, remove the `home` section and deploy; the Home tab disappears.

### A Cloudflare Tunnel, if you don't use Nabu Casa

A tunnel makes Home Assistant reachable from the Worker without opening a port on your router. The
steps below are the usual ones; Cloudflare's tunnel guide and Home Assistant's reverse-proxy notes are
the reference if your setup differs.

1. In the Cloudflare dashboard open **Zero Trust → Networks → Tunnels → Create a tunnel**, and run the
   connector it shows on a machine on the same network as Home Assistant. The **Cloudflared** add-on
   in Home Assistant does this, or run `cloudflared` on another machine.
2. Add a **public hostname**, for example `ha.example.com` (a domain on your Cloudflare account), with
   the service **HTTP** and the address `http://<Home Assistant's address>:8123`. Use that hostname
   as `home.url`.
3. Tell Home Assistant to trust the tunnel's connection, or it answers `400 Bad Request`. In
   `configuration.yaml`, then restart Home Assistant:

   ```yaml
   http:
     use_x_forwarded_for: true
     trusted_proxies:
       - 127.0.0.1          # the address the connector connects from: the add-on's network
       - 172.30.33.0/24     # or the machine's address, whichever Home Assistant logs as refused
   ```

4. Don't put Cloudflare Access in front of it, or let `/api/` through (see step 1 above).
5. Check it, with a token from the steps above:

   ```bash
   curl -H "Authorization: Bearer $HA_TOKEN" https://ha.example.com/api/
   ```

   It should answer `{"message":"API running."}`.

### What each entity becomes

| `settings.json` entry | Tile | Service calls |
| --- | --- | --- |
| `light.*`, `switch.*` | Light (dimmable when the light supports brightness) | `turn_on` (`brightness_pct`), `turn_off` |
| `cover.*` | Blind; `"garage": true` or `device_class: garage` makes it a garage door | `set_cover_position`, `open_cover`, `close_cover` |
| `lock.*` | Lock (hold to unlock, tap to lock) | `lock`, `unlock` |
| `climate` | Thermostat panel | `set_temperature`, `set_hvac_mode` |
| every `media_player.*` | The **Media** tab: one card per player, playing first | `media_play_pause`, `media_previous_track`, `media_next_track`, `volume_set`, `volume_mute`, `turn_on`, `turn_off` |
| `scenes` | Scene buttons | `scene.turn_on` |
| `cameras` | Snapshots, refreshed every 20 s while the Home tab is open. The Worker keeps the last good picture of each camera and sends that if Home Assistant can't get a new one (a sleeping or busy camera makes it answer "500" after 10 s), so the frame never shows a broken image; the age counter shows how old it is | `camera_proxy` |

The **Media** tab finds every media player by itself. Integrations often register one speaker
several times (Cast and Assistant, old TV entries), so players are merged by name, keeping the most
active, and unavailable ones stay hidden until they're back. To tidy the list, add
`"players": { "exclude": ["media_player.x"], "names": { "media_player.y": "Kitchen" } }` to `home`.
Controls appear only where the player supports them.

**Push the states (recommended).** By default the Worker reads Home Assistant's whole state list
(about 800 KB here) to find the few dozen entities it shows, and that parsing costs more Worker
CPU than Cloudflare's free plan allows (10 ms; a refresh takes 14–22). Let Home Assistant send the
Worker just those states instead, on every change and once a minute:

```bash
npm run ha-state -- --with-token
```

prints a `secrets.yaml` line, a `rest_command` and an automation to paste into Home Assistant, using
the same `ALERT_TOKEN` as the pushed alerts. After a **full restart** of Home Assistant (a YAML
reload doesn't register a new `rest_command`), **Status** in the display controls says
"HA connected (pushed)" and each Home request takes 1–3 ms. If the pushes stop (Home Assistant
restarting, the automation removed) for 2½ minutes, or you control something from the dashboard,
the Worker reads Home Assistant itself until the next push, so nothing breaks and it says
"(read by the Worker)" instead. The pushed states are kept in the dashboard's database, including
people's positions and media artwork addresses, like the cache. To check the template first, paste
the `payload` into Home Assistant's Developer tools → Template.

*Known limit:* the template uses `expand()`, which replaces a group or light group with its
members, so a group entity listed in `settings.json` would not be pushed (untested; none of the
author's entities is a group). List the members instead, or leave the push off, if you use groups.

`alerts` turns Home Assistant state into full-screen alerts on the frame. Each entry in it is one kind:

- `doorbell`: when the entity changes (a binary sensor turning on, or an `event.` entity), show the
  camera with hold-to-unlock. Add `"off": true` inside it to turn the doorbell off for now (no
  alert, no camera takeover, and `npm run ha-alerts` / `ha-state` leave it out) while keeping its
  settings; delete that word to turn it back on, and run `npm run ha-alerts` again if you pasted the
  automation.
- `leak`: when any listed sensor turns on, take over the screen. If `valve` is set (a `valve.*` or a
  `switch.*`), hold to shut off the water.
- `smoke`: when any listed smoke or carbon monoxide detector turns on, take over the screen and name
  the detectors going off. Detectors with `co` in their entity id, or with Home Assistant's
  `carbon_monoxide` device class, get the carbon monoxide wording. The frame only shows the alert;
  it doesn't replace the alarms themselves.
- `garageOpenMinutes`: show a banner when the garage has been open this long.

**How alerts arrive.** Without help, the frames poll Home
Assistant through the Worker every 30 seconds (every 60 on the photo screen and at night), and the
Worker shares one read for 8 s, so without help an alert can take half a minute. **Pushed alerts** make it
a couple of seconds: put a random `ALERT_TOKEN` in `secrets.json` (`openssl rand -hex 16`), run
`npm run secrets`, then `npm run ha-alerts -- --with-token` prints a `secrets.yaml` line, a
`rest_command` and an automation for your doorbell, smoke and leak entities to paste into Home
Assistant. It calls `POST /api/alert` the moment one changes. Each frame keeps one WebSocket
open to the Worker (a Cloudflare Durable Object, `AlertHub`), which sends the alert down every
connection at once, so the frame hears within a moment and refreshes straight away, and nothing
polls while it waits. A frame pings every 30 s and reconnects by itself if the connection drops;
while it's down it checks the counter every 10 s, so an alert is never missed. The token can only
do that (and push the states, below), not read the dashboard.

`energy` adds a line to the Home card: `cost` is a sensor (or a list, added up) with today's
spend, such as a smart meter's electricity and gas cost today. With `power` (W or kW now), the
line switches to the current draw while it's above `high` watts (3000 by default).

With `power` set, the Home card's line shows **electricity so far today** (worked out from the
power sensor's history) and the Home tab gets an **Energy** card; tapping either opens **Energy ·
today so far**: kWh per hour as bars, now, today's peak, and the same point yesterday. Add `usage`
(a smart meter's "usage today" sensors) and the panel says when the meter last reported: meter
data collected via the DCC usually arrives hours late. `rate` (£/kWh) and `standing` (£/day) add
an estimated cost.

The Home card's second line is an **all clear**: it names the first thing that needs a look (a
door unlocked, the garage open, the water main off, leak or smoke sensors that have stopped
reporting, or a low battery on one of them) and reads "All clear" otherwise. Tap it for the list.
A battery counts when Home Assistant has a `sensor.<name>_battery` whose `<name>` starts one of the
alert entities (Zigbee sensors are usually named this way).

`display.awayOffMinutes` (top level of `settings.json`) turns the screen off once everyone Home
Assistant can place has been away that long, and back on when someone comes home, taps it, or an
alert fires. People whose state is unknown are left out. With kiosk 1.2 the backlight goes down
too; Android keeps running so the frame can wake by itself.

**Phone notifications when a frame goes quiet** (needs Home Assistant): list Home Assistant notify
services in `settings.json`, for example `"frames": { "notify": ["notify.mobile_app_pixel"] }`
(Home Assistant: **Developer tools → Actions**, search `notify.mobile_app`). Every 5 minutes the
Worker checks the list; a frame quiet for `quietMinutes` (20 by default, never less) gets one
"offline" notification and a "back" one when it checks in again, which replaces it on Android. It
goes through the same `HA_TOKEN` as everything else; a non-admin user may call notify. If the
house's internet is down, the Worker can't reach Home Assistant either, so that outage goes
unreported. **Test alerts → Frame notification** in the display controls sends a test.

## How it fits together

- `worker/index.js` contains the API, auth, D1/R2 access and the Home Assistant mapping.
  `worker/ics.js` expands recurring events with [ical.js](https://github.com/kewisch/ical.js), and
  handles exceptions, moved occurrences, cancellations and time zones.
- `js/sync.js` loads data and keeps it fresh: lists every 30 s, calendar every 5 min, weather every
  15 min, Home Assistant every 30 s (60 s in ambient and night mode), pushed alerts by WebSocket. It
  saves the last copy on the
  device, so the frame starts instantly and keeps showing something during an outage. Changes on
  screen are optimistic and saved in the background. If a save fails, the frame says so and reloads
  the true state.
- Parsed calendars and the forecast are cached in D1 (10 and 15 minutes), so the feeds aren't
  fetched on every poll. If a feed fails, the last good copy is shown.
- Calendars are read through their iCal links. Events added with **+** on the frame are stored in
  D1, and written to Google Calendar too when a service account is set up.
- The Worker reads Home Assistant's `/api/states` once and shares the result (and the finished
  Home view) between all screens for 8 s, so frames, phones and the energy panel cost one read
  between them. Details in [DEVLOG.md](DEVLOG.md).

## Local development

```bash
cp settings.example.json settings.json                               # then edit
cp secrets.example.json secrets.json                                 # then edit
npm run secrets -- --local                                           # writes .dev.vars, uploads nothing
npm run db:migrate:local
npm run build && npx wrangler dev --port 8787
```

Open `http://localhost:8787/?key=<DASHBOARD_KEY>`. For live UI reloading, keep `wrangler dev` running and also
run `npm run dev`. Vite proxies `/api` to it, and the cookie works on both ports.

## Costs and limits

Cloudflare's free plan allows 100,000 Worker requests a day, and every request counts, pages and
images included (`run_worker_first`). A frame with Home Assistant makes about 10,000 a day: Home
Assistant and lists every 30 s in the day, and one WebSocket that stays open (it counts once, plus a handful of pings).
Without Home Assistant it's about 3,500. A phone adds a few hundred while the page is open. So
several frames fit in the free plan; past the limit, requests fail until midnight UTC. Usage is
under **Workers & Pages → home-hq → Metrics** in the Cloudflare dashboard. D1 and R2 use stay well
inside their free allowances.

A calendar with years of recurring events can exceed the free plan's 10 ms CPU limit when it's
re-parsed (once every 10 minutes at most). If you see `exceeded CPU` errors in
`npx wrangler tail`, use fewer or smaller calendars, or Workers Paid.

Google: drive times are capped at 300 Routes API calls a day (see [Calendars](#calendars)); the
Calendar API has no charge.
