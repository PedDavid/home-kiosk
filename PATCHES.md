# Local patches to the vendored frontend

Keep this list short. Every patch here has to be carried across `git subtree pull`.

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |
| Screenshot tests | `frontend/tests/screens/`, `frontend/playwright.config.mjs`, `frontend/package.json`, `frontend/package-lock.json` | Playwright screenshots and a layout check of the wall view at the frame's 1280×800, against mocked API fixtures; test tooling only, the app is unchanged |
| Drive wording | `frontend/js/views/calendar.js`, `frontend/js/app.js` (comment) | The event sheet said "live traffic from …’s location"; drive times now come from OSRM (no traffic) from home or a set origin (#18), so it says "from home" / "from Alex’s place" |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
