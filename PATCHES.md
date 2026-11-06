# Local patches to the vendored frontend

Keep this list short. Every patch here has to be carried across `git subtree pull`.

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |
| Quiet photo frame | `frontend/js/app.js`, `frontend/js/views/photos.js`, `frontend/js/views/ambient.js`, `frontend/css/views/ambient.css`, `frontend/js/views/notes.js`, `frontend/css/views/notes.css` | After eight idle seconds in Photos, show an edge-to-edge image without overlays; first tap restores photo controls. Daytime ambient mode is also image-only; night/off schedules remain active. |
| Immich device pairing | `frontend/immich.html`, `frontend/js/immich-auth.js`, `frontend/js/sync.js`, `frontend/js/views/photos.js`, `frontend/css/views/photos.css`, `frontend/vite.config.js` | QR approval page and connection link; issued credentials remain on the backend. |
| Offline fonts | `frontend/css/fonts.css`, `frontend/public/fonts/`, `frontend/index.html`, `frontend/m.html` | Serve Archivo and Instrument Serif locally so the LAN-only frame uses the intended type metrics; font licences are bundled |
| Weather location in Display Controls | `frontend/js/system.js`, `frontend/js/sync.js`, `frontend/css/base.css` | Show the automatically resolved city and configured coordinates, with a locally generated QR code to view the map on a phone. Reuses the bundled QR renderer in `frontend/js/share.js`. |
| Frame grid sizing | `frontend/css/base.css`, `frontend/css/views/home.css` | Prevent intrinsic card widths from expanding the home grid beyond the 1280×800 frame |
| Offline layout regression check | `frontend/tools/check-layout.mjs`, `frontend/package.json`, `frontend/package-lock.json` | Capture frame-sized screenshots and check font loading, clock spacing and card bounds without deployment |
| Preserve photo colours at night | `frontend/css/base.css`, `frontend/js/app.js`, `frontend/tools/check-photo-colours.mjs`, `frontend/package.json` | Apply the night filter to UI regions rather than the shared view container, preserving slideshow and home-tile image colours. |
| Explicit photo full screen | `frontend/js/icons.js`, `frontend/js/app.js`, `frontend/js/device.js`, `frontend/js/views/photos.js`, `frontend/css/views/photos.css`, `frontend/tools/check-photo-fullscreen.mjs`, `frontend/package.json` | Keep the current slide/layout with only an exit control; suppress automatic sleep and reloads until exit, with native full-screen and wake-lock support where available. |
| Screenshot tests | `frontend/tests/screens/`, `frontend/playwright.config.mjs`, `frontend/package.json`, `frontend/package-lock.json` | Playwright screenshots and a layout check of the wall view at the frame's 1280×800, against mocked API fixtures; test tooling only, the app is unchanged |
| Share photo as a QR code (#21) | `frontend/js/share.js`, `frontend/css/views/share.css` (new); two lines in `frontend/js/views/photos.js`, one each in `frontend/js/main.js` and `frontend/index.html`; `qrcode-generator` in `frontend/package.json` and `frontend/package-lock.json` | An icon-only Share button beside the full-screen button (also in full screen) shows a QR code that opens the photo in Immich on a phone. Shown only for photos with a `shareUrl` from `/api/state` |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
