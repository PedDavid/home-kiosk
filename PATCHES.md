# Local patches to the vendored frontend

Keep this list short. Every patch here has to be carried across `git subtree pull`.

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |
| Quiet photo frame | `frontend/js/app.js`, `frontend/js/views/photos.js`, `frontend/js/views/ambient.js`, `frontend/css/views/ambient.css`, `frontend/js/views/notes.js`, `frontend/css/views/notes.css` | After eight idle seconds in Photos, show an edge-to-edge image without overlays; first tap restores photo controls. Daytime ambient mode is also image-only; night/off schedules remain active. |
| Immich device pairing | `frontend/immich.html`, `frontend/js/immich-auth.js`, `frontend/js/sync.js`, `frontend/js/views/photos.js`, `frontend/css/views/photos.css`, `frontend/vite.config.js` | QR approval page and connection link; issued credentials remain on the backend. |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
