#!/bin/bash
# sd_shots.sh - real screenshots of SolidDraft for the site. Nothing here is rendered or
# mocked: the desktop app draws its own window, the browser app runs at its public address.
#   APP=/path/to/soliddraft-app PARTS=site/parts OUT=app/public/assets/app ./sd_shots.sh
# Needs a Wayland or X session for the desktop app. On Hyprland the window is tiled unless a
# rule floats it; the rule below is added at runtime and lasts until logout.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); SITE=$(cd "$HERE/../.." && pwd)
APP=${APP:-soliddraft-app}; PARTS=${PARTS:-$SITE/parts}; OUT=${OUT:-$SITE/app/public/assets/app}
RAW=${RAW:-/tmp/sd-shots}; mkdir -p "$OUT" "$RAW"
if command -v hyprctl >/dev/null && [ -d /run/user/$(id -u)/hypr ]; then
  export HYPRLAND_INSTANCE_SIGNATURE=${HYPRLAND_INSTANCE_SIGNATURE:-$(ls /run/user/$(id -u)/hypr | head -1)}
  hyprctl eval 'o.window({ class = "^soliddraft$" }, { float = true, center = true, size = { 1600, 1000 } })' >/dev/null || true
fi
shot() { # file, name, theme
  timeout 90 "$APP" "$1" --screenshot "$RAW/$2.png" --size 1600x1000 --no-assistant --preset studio --theme "$3" --after 60
  magick "$RAW/$2.png" -resize 1600x -quality 84 "$OUT/$2.webp"
}
shot "$PARTS/bracket-plain.dwprt" desktop-bracket-dark dark
shot "$PARTS/bracket-plain.dwprt" desktop-bracket-light light
shot "$PARTS/csk-plate.dwprt" desktop-plate-dark dark
shot "$PARTS/rounded-boss.dwprt" desktop-boss-dark dark
shot "$PARTS/collar.dwprt" desktop-collar-dark dark
# The drawing: made with `soliddraft draw bracket-plain.dwprt --template iso-a3 --out bracket.dwdrw`.
[ -f "$PARTS/bracket.dwdrw" ] && shot "$PARTS/bracket.dwdrw" desktop-drawing dark
# The browser app, headless Chromium with the GPU (Playwright from the mise install).
node "$HERE/../web/shot-sd.mjs" 'https://sd.zigrawbohnbawerick.org/?demo' "$RAW/browser-demo.png" 1600 1000 15000
magick "$RAW/browser-demo.png" -quality 84 "$OUT/browser-demo.webp"
# The browser app with a real part: Ctrl+O opens its file chooser, Playwright feeds the file.
node "$HERE/../web/upload-sd.mjs" "$PARTS/bracket-plain.dwprt" "$RAW/browser-bracket.png"
magick "$RAW/browser-bracket.png" -quality 84 "$OUT/browser-bracket.webp"
