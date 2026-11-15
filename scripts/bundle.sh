#!/bin/sh
# Build Neolink.app: universal binary, icon, Info.plist, ad-hoc signature.
#
#   scripts/bundle.sh [--release] [--install]
#
# No vendor bytes are embedded. The bundle only carries our own executable and
# resources; DisplayLink Manager is fetched at runtime by `neolinkctl vendor`.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
profile=debug
install=0
for arg in "$@"; do
  case "$arg" in
    --release) profile=release ;;
    --install) install=1 ;;
    *) echo "unknown flag: $arg" >&2; exit 2 ;;
  esac
done

out="$root/target/$profile/Neolink.app"
rm -rf "$out"
mkdir -p "$out/Contents/MacOS" "$out/Contents/Resources"

# Universal, so the same bundle runs on Intel and Apple silicon docks hosts.
if rustup target list --installed 2>/dev/null | grep -qx aarch64-apple-darwin &&
   rustup target list --installed 2>/dev/null | grep -qx x86_64-apple-darwin; then
  cargo build -p neolink --profile "$profile" --target aarch64-apple-darwin
  cargo build -p neolink --profile "$profile" --target x86_64-apple-darwin
  lipo -create \
    "$root/target/aarch64-apple-darwin/$profile/neolink" \
    "$root/target/x86_64-apple-darwin/$profile/neolink" \
    -output "$out/Contents/MacOS/neolink"
  cargo build -p neolink-core --profile "$profile" --target aarch64-apple-darwin --bin neolinkctl
  cargo build -p neolink-core --profile "$profile" --target x86_64-apple-darwin --bin neolinkctl
  lipo -create \
    "$root/target/aarch64-apple-darwin/$profile/neolinkctl" \
    "$root/target/x86_64-apple-darwin/$profile/neolinkctl" \
    -output "$out/Contents/MacOS/neolinkctl"
else
  cargo build -p neolink --profile "$profile"
  cargo build -p neolink-core --profile "$profile" --bin neolinkctl
  cp "$root/target/$profile/neolink" "$out/Contents/MacOS/neolink"
  cp "$root/target/$profile/neolinkctl" "$out/Contents/MacOS/neolinkctl"
fi

cp "$root/packaging/Info.plist" "$out/Contents/Info.plist"
cp "$root/packaging/Neolink.icns" "$out/Contents/Resources/Neolink.icns"
printf 'APPL????' > "$out/Contents/PkgInfo"

# Ad-hoc signature: enough for Gatekeeper to launch a locally built bundle.
# Replace with a Developer ID identity + notarytool for distribution.
codesign --force --strict --deep --sign - "$out"

echo "built $out"
if [ "$install" = 1 ]; then
  rm -rf "/Applications/Neolink.app"
  cp -R "$out" /Applications/
  echo "installed /Applications/Neolink.app"
fi
