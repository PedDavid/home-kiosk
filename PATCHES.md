# Local patches to the vendored frontend

Keep this list short. Every patch here has to be carried across `git subtree pull`.

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |
| Screenshot tests | `frontend/tests/screens/`, `frontend/playwright.config.mjs`, `frontend/package.json`, `frontend/package-lock.json` | Playwright screenshots and a layout check of the wall view at the frame's 1280×800, against mocked API fixtures; test tooling only, the app is unchanged |
| Hide "+" (add event) when the calendar can't take writes | `frontend/js/sync.js`, `frontend/js/views/calendar.js` | `/api/state` reports `capabilities.addEvents`; with only read-only (ICS) calendars the frame shouldn't offer to add events. Marked `// home-kiosk patch` |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
