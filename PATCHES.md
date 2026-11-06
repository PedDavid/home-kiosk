# Local patches to the vendored frontend

Keep this list short. Every patch here has to be carried across `git subtree pull`.

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |
| Screenshot tests | `frontend/tests/screens/`, `frontend/playwright.config.mjs`, `frontend/package.json`, `frontend/package-lock.json` | Playwright screenshots and a layout check of the wall view at the frame's 1280×800, against mocked API fixtures; test tooling only, the app is unchanged |
| Hide "+" (add event) when the calendar can't take writes | `frontend/js/sync.js`, `frontend/js/views/calendar.js` | `/api/state` reports `capabilities.addEvents`; with only read-only (ICS) calendars the frame shouldn't offer to add events. Marked `// home-kiosk patch` |
| Undo the optimistic change after a failed write | `frontend/js/sync.js` (`App.write`, `refresh`) | After "Couldn't save", `App.write` refreshes the source, but that refresh was dropped twice: it starts in the same millisecond as the write is marked (`wrote >= started`, now `>`), and the server's unchanged copy equals the last one applied, so `apply` skipped it (`last[source]` is now forgotten on failure). An event that failed to save stayed in `DATA.events` ("next up", the other views) until the server's data next changed. An open calendar view still keeps drawing it until its next redraw (upstream holds redraws back for 6 s after a touch) |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
