#!/usr/bin/env bash
# Regenerate every app icon from assets/app-icon/localwriter.svg.
#
# Needs: resvg (brew install resvg / cargo install resvg). On macOS, iconutil also writes the
# .icns. The outputs are committed, so builds and packaging never need these tools.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
SVG="$DIR/localwriter.svg"
ID="com.local.localwriter"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v resvg >/dev/null || { echo "error: resvg not found (brew install resvg)" >&2; exit 1; }

# The artwork is a full-bleed tile on a 100-unit grid (viewBox 0 0 100 100, rx=22; the width/height
# of 1024 only set its default size). Windows and Linux use it as is. macOS icons sit on Apple's
# grid: an 824/1024 body with a transparent margin, so widen the viewBox by 1024/824: a 12.136 margin
# on each side, 124.272 wide.
MAC="$TMP/mac.svg"
sed 's/viewBox="0 0 100 100"/viewBox="-12.1359 -12.1359 124.2718 124.2718"/' "$SVG" >"$MAC"
grep -q 'viewBox="-12.1359' "$MAC" || { echo "error: expected viewBox=\"0 0 100 100\" in $SVG" >&2; exit 1; }

render() {
  local src="$1" size="$2" dst="$3"
  resvg -w "$size" -h "$size" "$src" "$dst" </dev/null
  return 0
}

render "$SVG" 1024 "$DIR/localwriter-1024.png"
# Runtime window/Dock icon on macOS (embedded by apps/localwriter); other platforms use hicolor 256.
render "$MAC" 512 "$DIR/localwriter-macos-512.png"

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
(cd "$ROOT" && cargo run -q -p xtask -- ico "$DIR/localwriter.ico" "${ICO_PNGS[@]}")

# Windows .ico per document format (assets/file-icons): 16/20/24 from the -16 drawing, 32/40 from
# the -32 one, 48 and up from the master. packaging/windows embeds them in localwriter.exe.
FI="$ROOT/assets/file-icons"
for master in "$FI"/*.svg; do
  ext="$(basename "$master" .svg)"
  case "$ext" in *-16 | *-32) continue ;; esac
  FI_PNGS=()
  for s in 16 20 24; do render "$FI/$ext-16.svg" "$s" "$TMP/$ext-$s.png"; FI_PNGS+=("$TMP/$ext-$s.png"); done
  for s in 32 40; do render "$FI/$ext-32.svg" "$s" "$TMP/$ext-$s.png"; FI_PNGS+=("$TMP/$ext-$s.png"); done
  for s in 48 64 128 256; do render "$master" "$s" "$TMP/$ext-$s.png"; FI_PNGS+=("$TMP/$ext-$s.png"); done
  (cd "$ROOT" && cargo run -q -p xtask -- ico "$FI/$ext.ico" "${FI_PNGS[@]}")
done

# macOS .icns.
if command -v iconutil >/dev/null; then
  SET="$TMP/localwriter.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    render "$MAC" "$s" "$SET/icon_${s}x${s}.png"
    render "$MAC" $((s * 2)) "$SET/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns -o "$DIR/localwriter.icns" "$SET"
else
  echo "warning: iconutil not found (macOS only); localwriter.icns not regenerated" >&2
fi
echo "icons written to $DIR"
