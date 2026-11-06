#!/bin/sh
# Pushes PNGs to the orphan branch screenshots/pr-<N> and prints Markdown that embeds them, for
# a pull request's description or a comment. Nothing in the working tree or index changes: the
# commit is built with git plumbing and has no parent, so the branch never touches main.
#   tests/screens/publish-pr.sh 42                          the baselines (tests/screens/screenshots)
#   tests/screens/publish-pr.sh 42 path/to/a.png dir/ ...   chosen files or folders
# Re-running replaces the branch. Delete it once the PR is merged or closed:
#   git push origin --delete screenshots/pr-42
set -eu
pr="${1:?usage: publish-pr.sh <PR number> [png or folder ...]}"
shift
here="$(cd "$(dirname "$0")" && pwd)"
[ $# -gt 0 ] || set -- "$here/screenshots"
repo="${SCREENS_REPO:-$(git remote get-url origin | sed -E 's#^.*github\.com[:/]##; s#^.*/git/##; s#\.git$##')}"
branch="screenshots/pr-$pr"

index="$(mktemp)"
trap 'rm -f "$index"' EXIT
rm -f "$index"
names=""
for arg in "$@"; do
  if [ -d "$arg" ]; then files=$(find "$arg" -maxdepth 1 -name '*.png' | sort); else files="$arg"; fi
  for f in $files; do
    name="$(basename "$f")"
    blob="$(git hash-object -w "$f")"
    GIT_INDEX_FILE="$index" git update-index --add --cacheinfo "100644,$blob,$name"
    names="$names $name"
  done
done
[ -n "$names" ] || { echo "no PNGs found in: $*" >&2; exit 1; }
tree="$(GIT_INDEX_FILE="$index" git write-tree)"
commit="$(git commit-tree "$tree" -m "Screenshots for #$pr")"
git push --force origin "$commit:refs/heads/$branch" >&2

echo
for name in $names; do
  echo "**${name%.png}**"
  echo
  echo "![${name%.png}](https://raw.githubusercontent.com/$repo/$branch/$name)"
  echo
done
