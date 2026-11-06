# Local patches to the vendored frontend

Keep this list short. Every patch here has to be carried across `git subtree pull`.

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |
| Share photo as a QR code (#21) | `js/share.js`, `css/views/share.css` (new); one line each in `js/views/photos.js`, `js/main.js`, `index.html`; `qrcode-generator` in `package.json` | Opens the photo shown in Immich on a phone. Shown only for photos with a `shareUrl` from `/api/state` |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
