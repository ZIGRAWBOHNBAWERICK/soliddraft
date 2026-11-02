#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
web="$(realpath "$1")"
test -f "$web/index.html"
: "${SITE_BASE:?}"
: "${VITE_SOLIDDRAFT_VERSION:?}"
: "${VITE_SOLIDDRAFT_PRERELEASE:?}"

( cd "$root/site/app" && bun install --frozen-lockfile && bun run lint && bun run test && bun run build:pages )
mkdir -p "$root/site/app/dist/pages/app"
cp -R "$web/." "$root/site/app/dist/pages/app/"
test -f "$root/site/app/dist/pages/index.html"
test -f "$root/site/app/dist/pages/app/index.html"
# Static hosting runs the kernel locally, with no assistant relay or bundled key.
! grep -rl "sk-ant-" "$root/site/app/dist/pages/app" >/dev/null
