# Local patches to vendored code

Keep these lists short. Every patch here has to be carried across `git subtree pull`.

## `frontend/` (sambartle/photo-frame-dashboard, `dashboard/`)

| Patch | Files | Why |
|---|---|---|
| `frontend/LICENSE` added | `frontend/LICENSE` | Upstream keeps its MIT licence at the repo root, outside `dashboard/` |

## `android/` (wesbos/photo-frame-dashboard, `dashboard/kiosk`)

| Patch | Files | Why |
|---|---|---|
| Pull down from the top edge to open the controls | `android/src/com/wesbos/homehq/MainActivity.java` (`dispatchTouchEvent`, `PULL_*`), `android/src/com/wesbos/homehq/PullBubble.java` (new), `android/README.md` (*Pull down from the top*) | The PF1007L's gslX680 touchscreen merges two-finger touches, so the stock two-finger hold is unreliable, and the Menu button alternative sits on the dashboard. A one-finger pull with a bubble that fills as you pull needs no on-screen control |

Planned: hide tabs whose capability `/api/state` reports as unavailable (PRD 002).
