#!/usr/bin/env bash
# Regenerate every app icon from assets/app-icon/localoffice.svg.
#
# Needs: resvg (brew install resvg / cargo install resvg). On macOS, iconutil also writes the
# .icns. The outputs are committed, so builds and packaging never need these tools.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
SVG="$DIR/localoffice.svg"
ID="com.local.localoffice"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v resvg >/dev/null || { echo "error: resvg not found (brew install resvg)" >&2; exit 1; }

# The artwork is a full-bleed 1024 tile (rx=224). Windows and Linux use it as is. macOS icons sit on
# Apple's grid: an 824/1024 body with a transparent margin, so widen the viewBox by 1024*1024/824.
MAC="$TMP/mac.svg"
sed 's/viewBox="0 0 1024 1024"/viewBox="-124.272 -124.272 1272.544 1272.544"/' "$SVG" >"$MAC"
grep -q 'viewBox="-124.272' "$MAC" || { echo "error: expected viewBox=\"0 0 1024 1024\" in $SVG" >&2; exit 1; }

render() {
  local src="$1" size="$2" dst="$3"
  resvg -w "$size" -h "$size" "$src" "$dst" </dev/null
  return 0
}

render "$SVG" 1024 "$DIR/localoffice-1024.png"
# Runtime window/Dock icon on macOS (embedded by apps/localoffice); other platforms use hicolor 256.
render "$MAC" 512 "$DIR/localoffice-macos-512.png"

# Linux hicolor theme.
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$DIR/hicolor/${s}x${s}/apps"
  render "$SVG" "$s" "$DIR/hicolor/${s}x${s}/apps/$ID.png"
done
mkdir -p "$DIR/hicolor/scalable/apps"
cp "$SVG" "$DIR/hicolor/scalable/apps/$ID.svg"

# Windows .ico.
ICO_PNGS=()
for s in 16 20 24 32 40 48 64 128 256; do
  render "$SVG" "$s" "$TMP/ico-$s.png"
  ICO_PNGS+=("$TMP/ico-$s.png")
done
(cd "$ROOT" && cargo run -q -p xtask -- ico "$DIR/localoffice.ico" "${ICO_PNGS[@]}")

# macOS .icns.
if command -v iconutil >/dev/null; then
  SET="$TMP/localoffice.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    render "$MAC" "$s" "$SET/icon_${s}x${s}.png"
    render "$MAC" $((s * 2)) "$SET/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns -o "$DIR/localoffice.icns" "$SET"
else
  echo "warning: iconutil not found (macOS only); localoffice.icns not regenerated" >&2
fi
echo "icons written to $DIR"
