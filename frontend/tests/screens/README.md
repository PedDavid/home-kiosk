# Screenshot tests

Playwright tests that render the wall view as the PF1007L frame shows it (1280×800, 1:1 pixels,
touch, Europe/Lisbon). They compare each screen with the PNGs in [`screenshots/`](screenshots/) and
check that nothing spills out of the 1280×800 frame.

| Test | Screen |
|---|---|
| `home`, `calendar`, `shopping`, `control`, `media` | the rail's tabs (Today, Calendar, Shopping, Home, Media) |
| `photos-landscape` | Photos with only a landscape photo (Cover) |
| `photos-portrait`, `photos-portrait-polaroid` | Photos with only a portrait photo: Cover (fitted on a blurred fill) and Auto (polaroid) |
| `photos-idle` | the idle photo screen (ambient mode, as `App.rest()` leaves it) |
| `night` | the dim night clock |
| `note-sheet` | the New note sheet over Calendar |
| `weather-open-meteo` | the weather card and hourly strip on a real Open-Meteo forecast: the recorded snapshot in `tests/fixtures/weather/`, served as the backend serves it, at noon on the snapshot's day |
| `home layout` | the layout check on Today: its right column must fit inside the 1280×800 frame |

## How the page is set up

[`kiosk.mjs`](kiosk.mjs) loads the built frontend (`dist/`) at a fake origin
(`https://home-hq.test`). No server runs:

- **API**: every `/api` call is answered from [`fixtures/`](fixtures/). These are the backend's
  demo responses (`src/demo.rs`), rendered at the pinned moment by
  `cargo run --example screen_fixtures`. CI fails if they're out of date. Writes get
  `{"ok":true}`, and the alerts WebSocket answers pings. A test can override a fixture with
  `test.use({ api: { state: s => ({ ...s, photos: [...] }) } })`.
- **Clock**: `Date` is fixed at Thursday 5 November 2026, 5:30 PM (`NOW`), so it's day mode,
  inside the bins window and before the night hours. Timers still run.
- **Randomness**: `Math.random` is seeded, so the slideshow order, Ken Burns offsets and polaroid
  tilt are the same on every run.
- **Motion**: transitions and animations are off, with `reducedMotion: 'reduce'`, and
  `toHaveScreenshot` disables animations.
- **Network**: the frame is LAN-only, so any request outside the fake origin fails the test. The
  Google Fonts stylesheet in `index.html` is answered from the pinned `@fontsource` packages, so
  text is drawn in Archivo and Instrument Serif. A build that serves its own fonts never makes
  that request.
- Errors thrown in the page fail the test.

## Running

```bash
cd frontend
npm run test:screens:docker           # check, in CI's image (needs Docker)
npm run test:screens:docker:update    # rewrite screenshots/ in CI's image; commit the PNGs
npm run test:screens                  # check with the local Playwright browser
npm run test:screens:update
```

All four build `dist/` first. Pixels depend on the Chromium build, the fonts and the
antialiasing, so **baselines must come from `mcr.microsoft.com/playwright:v1.65.0-noble`**, the
image CI's `screens` job uses. The `:docker` scripts run there, and their results match CI. The
plain scripts are fine for quick runs and for the layout checks, but their screenshots can differ
from the baselines. Don't commit baselines from them. Behind a proxy, pass it through, for example
`DOCKER_FLAGS="--network host -e HTTPS_PROXY -e HTTP_PROXY"`. Without Docker, run the CI workflow
by hand with *update screenshots* ticked and take the PNGs from its `screenshots` artifact.

When a test fails, `test-results/screens/` has `*-actual.png` and `*-diff.png` for each mismatch.
CI uploads those, the HTML report and the baselines as the `screenshots` artifact. Bump
`@playwright/test`, the image tag in [`docker.sh`](docker.sh) and the CI job together, then
re-render the baselines.

## Adding a screen

Add a test to [`screens.spec.mjs`](screens.spec.mjs): `open(page, '<tab>')`, act through the UI
(or `App.*` for states such as modes that only time reaches), `settle(page)`, then
`await expect(page).toHaveScreenshot('<name>.png')` and `await fits(page)`. Render its baseline
with `npm run test:screens:docker:update` and commit it.

## Screenshots in a pull request (for agents too)

Any PR that changes what the frame shows should show it. None of this needs a push beyond the
PR's own branch:

1. Run `npm run test:screens:docker:update` and commit the changed PNGs in `screenshots/` on the
   PR branch. GitHub's diff then shows the before and after.
2. **CI links them for you.** On a pull request from this repository, the `screens` job keeps one
   comment up to date (marked `<!-- screens -->`). It holds the result, a link to the
   `screenshots` artifact (baselines, plus `-actual`/`-diff` PNGs on failure), and the baselines
   the PR adds or changes, embedded from its head commit. The run's summary has the same.
3. To embed them in the description as well, print the Markdown after pushing:

   ```bash
   tests/screens/pr-markdown.sh --changed     # baselines changed since origin/main
   tests/screens/pr-markdown.sh               # all baselines
   ```

   The links are `https://raw.githubusercontent.com/<owner>/<repo>/<commit>/<path>` and point at
   the pushed HEAD commit, so they keep showing that version.
4. For an image no test produces (a device photo, a one-off state), commit it to the PR branch
   under `.github/pr-screenshots/pr-<N>/` and link it the same way
   (`tests/screens/pr-markdown.sh ../.github/pr-screenshots/pr-<N>/*.png`). For a state the frame
   can reach, prefer adding a test (above), so the next change is checked against it.
