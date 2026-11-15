#!/bin/sh
# Build Neolink.dmg: a drag-to-Applications disk image with a designed background.
#
#   scripts/dmg.sh [--skip-build] [--app PATH] [--out PATH]
#
# Builds the release bundle (unless --skip-build or --app), renders the background from
# packaging/dmg/background.swift, and hands both to dmgbuild (packaging/dmg/settings.py),
# which writes the Finder layout and compresses the image.
# The first launch of the app runs the setup assistant, which provisions the DisplayLink
# driver; no vendor bytes go into the image.
#
# Signing: set CODESIGN_IDENTITY to a Developer ID to sign the image; set NOTARY_PROFILE
# (a `notarytool store-credentials` keychain profile) to notarise and staple it too.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
app=""
out=""
build=1
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) build=0 ;;
    --app) app=$2; build=0; shift ;;
    --out) out=$2; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
  shift
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n1)
[ -n "$app" ] || app="$root/target/release/Neolink.app"
[ -n "$out" ] || out="$root/target/release/Neolink_${version}_macos.dmg"

if [ "$build" = 1 ]; then
  "$root/scripts/bundle.sh" --release
fi
[ -d "$app" ] || { echo "no app bundle at $app" >&2; exit 1; }

work=$(mktemp -d "${TMPDIR:-/tmp}/neolink-dmg.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

# dmgbuild writes the Finder layout without driving Finder, so this runs headless on CI.
# It lives in a private venv under target/ so the build never touches system Python.
venv="$root/target/dmg-venv"
if [ ! -x "$venv/bin/dmgbuild" ]; then
  python3 -m venv "$venv"
  "$venv/bin/pip" install --quiet --disable-pip-version-check "dmgbuild==1.6.7"
fi

# Background: 1x and 2x merged into one TIFF so Retina displays get the sharp copy.
swift "$root/packaging/dmg/background.swift" "$root" "$work/bg" "$version" >/dev/null
tiffutil -cathidpicheck "$work/bg/background.png" "$work/bg/background@2x.png" \
  -out "$work/background.tiff" 2>/dev/null

mkdir -p "$(dirname "$out")"
rm -f "$out"
"$venv/bin/dmgbuild" \
  -s "$root/packaging/dmg/settings.py" \
  -D app="$app" \
  -D background="$work/background.tiff" \
  -D icns="$root/packaging/Neolink.icns" \
  "Neolink" "$out" 2>&1 | grep -v "is deprecated" >&2 || true
[ -f "$out" ] || { echo "dmgbuild failed" >&2; exit 1; }

if [ -n "${CODESIGN_IDENTITY:-}" ]; then
  codesign --force --sign "$CODESIGN_IDENTITY" --timestamp "$out"
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$out" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$out"
  fi
fi

shasum -a 256 "$out" | cut -d' ' -f1 > "$out.sha256"
echo "built $out ($(du -h "$out" | cut -f1 | tr -d ' '))"
