# Local patches to the vendored frontend

Keep this list short. Every patch here has to be carried across `git subtree pull`.

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |
| Screenshot tests | `frontend/tests/screens/`, `frontend/playwright.config.mjs`, `frontend/package.json`, `frontend/package-lock.json` | Playwright screenshots and a layout check of the wall view at the frame's 1280×800, against mocked API fixtures; test tooling only, the app is unchanged |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
