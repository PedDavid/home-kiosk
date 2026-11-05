# Local patches to the vendored frontend

Keep this list short. Every patch here has to be carried across `git subtree pull`.

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |
| Offline fonts | `frontend/css/fonts.css`, `frontend/public/fonts/`, `frontend/index.html`, `frontend/m.html` | Serve Archivo and Instrument Serif locally so the LAN-only frame uses the intended type metrics; font licences are bundled |
| Frame grid sizing | `frontend/css/base.css`, `frontend/css/views/home.css` | Prevent intrinsic card widths from expanding the home grid beyond the 1280×800 frame |
| Offline layout regression check | `frontend/tools/check-layout.mjs`, `frontend/package.json`, `frontend/package-lock.json` | Capture frame-sized screenshots and check font loading, clock spacing and card bounds without deployment |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
