#!/bin/sh
# Prints Markdown that embeds screenshots already committed and pushed on the current branch,
# for a pull request's description. It pushes nothing: the links point at the PNGs in the HEAD
# commit (raw.githubusercontent.com/<repo>/<sha>/<path>), so they keep showing this version
# even after later pushes, and they work once HEAD is pushed.
#   tests/screens/pr-markdown.sh                        every baseline in tests/screens/screenshots
#   tests/screens/pr-markdown.sh --changed [base]       only the baselines changed since base (origin/main)
#   tests/screens/pr-markdown.sh path/to/a.png ...      chosen committed PNGs (paths from here)
set -eu
top="$(git rev-parse --show-toplevel)"
here="$(cd "$(dirname "$0")" && pwd)"
repo="${SCREENS_REPO:-$(git remote get-url origin | sed -E 's#^.*github\.com[:/]##; s#^.*/git/##; s#\.git$##')}"
sha="$(git rev-parse HEAD)"
rel() { d="$(cd "$(dirname "$1")" && pwd -P)"; t="$(cd "$top" && pwd -P)"; echo "${d#"$t"/}/$(basename "$1")"; }

if [ "${1:-}" = "--changed" ]; then
  files=$(git -C "$top" diff --name-only --diff-filter=AM "${2:-origin/main}...HEAD" -- "$(rel "$here/screenshots")" | sed "s#^#$top/#")
elif [ $# -gt 0 ]; then
  files="$*"
else
  files=$(find "$here/screenshots" -maxdepth 1 -name '*.png' | sort)
fi
[ -n "$files" ] || { echo "no screenshots to link" >&2; exit 1; }
git diff --quiet HEAD -- $files || echo "warning: some files differ from HEAD; commit and push them first" >&2
git branch -r --contains "$sha" 2>/dev/null | grep -q . || echo "warning: $sha is not pushed yet; the links work once it is" >&2

for f in $files; do
  path="$(rel "$f")"
  name="$(basename "$f" .png)"
  printf '**%s**\n\n![%s](https://raw.githubusercontent.com/%s/%s/%s)\n\n' "$name" "$name" "$repo" "$sha" "$path"
done
