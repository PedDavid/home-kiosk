#!/bin/sh
# Runs the screenshot tests in the same image as CI's `screens` job, so baselines rendered here
# match CI pixel for pixel (fonts, antialiasing and the Chromium build all come from the image).
#   tests/screens/docker.sh                       check (npm run test:screens:docker)
#   tests/screens/docker.sh --update-snapshots    rewrite the baselines (npm run test:screens:docker:update)
# Extra `docker run` flags, e.g. a proxy: DOCKER_FLAGS="--network host -e HTTPS_PROXY -e HTTP_PROXY"
# Keep the image tag in step with @playwright/test in package.json and the CI job.
set -eu
IMAGE="${SCREENS_IMAGE:-mcr.microsoft.com/playwright:v1.65.0-noble}"
cd "$(dirname "$0")/../.."
# node_modules lives in a volume: the host's may hold another platform's native binaries.
# shellcheck disable=SC2086
exec docker run --rm --init --ipc=host ${DOCKER_FLAGS:-} \
  -v "$PWD":/work -v home-kiosk-screens-node-modules:/work/node_modules -w /work \
  -e CI="${CI:-}" -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  "$IMAGE" sh -c '
    npm ci --no-audit --no-fund --loglevel=error && npx vite build --logLevel warn || exit 1
    npx playwright test "$@"; status=$?
    chown -R "$HOST_UID:$HOST_GID" dist test-results playwright-report tests/screens/screenshots 2>/dev/null
    exit $status' sh "$@"
