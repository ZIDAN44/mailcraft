#!/usr/bin/env bash
# Build, sign and (optionally) notarize the macOS release artifacts:
#
#   $DIST/mailcraft-<version>-macos-<arch>.dmg          MailCraft.app on a drag-to-Applications DMG
#   $DIST/mailcraft-cli-<version>-macos-<arch>.zip      the headless CLI
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build]
#
# Signing (env):
#   MACOS_SIGN_IDENTITY   codesign identity (name or SHA-1). Default "-" = ad-hoc (local testing;
#                         Gatekeeper will reject the result on other Macs). In CI, import-cert.sh sets it.
#   MACOS_KEYCHAIN        keychain holding the identity (optional)
# Notarization (env; all three needed, and a real identity):
#   APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID
set -euo pipefail
# shellcheck source=../common.sh
. "$(dirname "${BASH_SOURCE[0]}")/../common.sh"
HERE="$ROOT/packaging/macos"

ARCH=universal
SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/MailCraft.app"
DMG="$DIST/mailcraft-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/mailcraft-cli-$VERSION-macos-$ARCH.zip"

NOTARIZE=0
if [ "$IDENTITY" = "-" ]; then
  # A build meant to be unsigned (UNSIGNED_BUILD=1, the test release) notes it instead of warning.
  if [ "${UNSIGNED_BUILD:-}" = 1 ] && [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::notice::macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); artifacts are not notarized"
  else
    warn "macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); artifacts are not notarized"
  fi
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  NOTARIZE=1
else
  warn "macOS: APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID incomplete; signed but not notarized"
fi

echo "==> MailCraft $VERSION for macOS ($ARCH), identity: $IDENTITY, notarize: $NOTARIZE"

# ---- build -------------------------------------------------------------------------------------
if [ "$SKIP_BUILD" = 0 ]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --release --locked -p mailcraft -p mailcraft-cli "${args[@]}")
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin"
for bin in mailcraft mailcraft-cli; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$bin"); done
  lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  lipo -info "$WORK/bin/$bin"
done

# ---- signing helpers ---------------------------------------------------------------------------
sign() {
  # Hardened runtime + secure timestamp for a real identity; ad-hoc can't be timestamped.
  local ts=(--timestamp)
  [ "$IDENTITY" = "-" ] && ts=(--timestamp=none)
  local kc=()
  [ -n "${MACOS_KEYCHAIN:-}" ] && kc=(--keychain "$MACOS_KEYCHAIN")
  codesign --force --sign "$IDENTITY" ${kc[@]+"${kc[@]}"} "${ts[@]}" "$@"
}

notarize() {
  local file="$1" out id status
  echo "==> notarizing $(basename "$file") (this can take a few minutes)"
  out="$(xcrun notarytool submit "$file" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --timeout 1h --output-format json)" || true
  echo "$out"
  id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
  status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
  if [ "$status" != "Accepted" ]; then
    if [ -n "$id" ]; then
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" || true
    fi
    echo "error: notarization of $(basename "$file") failed (status: ${status:-unknown})" >&2
    exit 1
  fi
}

# Staples the notarization ticket: Apple's servers drop the connection now and then
# ("Could not validate ticket"). Usage: stapler_retry <path>; five tries with growing pauses.
stapler_retry() {
  local tries=0 pause
  until xcrun stapler staple "$1" && xcrun stapler validate "$1"; do
    tries=$((tries + 1))
    if [ "$tries" -ge 5 ]; then
      echo "error: Apple's ticket for $1 couldn't be stapled in $tries tries" >&2
      return 1
    fi
    pause=$((tries * 15))
    echo "the ticket for $1 didn't staple (try $tries); again in ${pause}s" >&2
    sleep "$pause"
  done
}

# ---- MailCraft.app ----------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Executable and icon carry the display name (CFBundleExecutable / CFBundleIconFile).
cp "$WORK/bin/mailcraft" "$APP/Contents/MacOS/MailCraft"
cp "$ROOT/assets/app-icon/mailcraft.icns" "$APP/Contents/Resources/MailCraft.icns"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${MAILCRAFT_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

# codesign --strict rejects extended attributes such as com.apple.FinderInfo or a resource fork
# ("Finder information, or similar detritus not allowed"); a file copied in can carry them. Clear
# them before signing; nothing after this point may write xattrs onto the bundle.
xattr -cr "$APP"

# Sign inside-out: nested code first, then the bundle itself (no --deep on the final signature).
# Today the only nested code is the main executable; frameworks/helpers would be signed here too.
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/MailCraft"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

if [ "$NOTARIZE" = 1 ]; then
  ditto -c -k --keepParent "$APP" "$WORK/MailCraft-notarize.zip"
  notarize "$WORK/MailCraft-notarize.zip"
  stapler_retry "$APP"
  spctl --assess --type execute -vvv "$APP"
fi

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/MailCraft.app"
ln -s /Applications "$STAGE/Applications"
# Finder window layout (background, icon positions), when packaging/macos/dmg/ provides one;
# without it Finder shows the two icons in its default layout.
if [ -f "$HERE/dmg/background.tiff" ] && [ -f "$HERE/dmg/dmg-layout.DS_Store" ]; then
  mkdir -p "$STAGE/.background"
  cp "$HERE/dmg/background.tiff" "$STAGE/.background/background.tiff"
  cp "$HERE/dmg/dmg-layout.DS_Store" "$STAGE/.DS_Store"
fi
rm -f "$DMG"
# Retries an hdiutil verb that attaches a device: on CI runners it fails now and then with
# "Resource busy". Usage: hdiutil_retry <verb> <args...>; five tries with growing pauses.
hdiutil_retry() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if hdiutil "$@"; then
      return 0
    fi
    if [ "$attempt" = 5 ]; then
      echo "error: hdiutil $1 failed 5 times" >&2
      return 1
    fi
    echo "hdiutil $1 failed (attempt $attempt of 5); retrying in $((attempt * 10))s" >&2
    sleep $((attempt * 10))
  done
}
# `hdiutil create -srcfolder` copies the staged files as they are. `makehybrid -hfs` + `convert`
# gave every file in the image a com.apple.FinderInfo, so the app in the DMG, and a copy installed
# from it, failed `codesign --verify --strict --deep` (as the other Craft apps found: WordCraft,
# PdfCraft, VectorCraft, DesignCraft). The volume name has no version: .DS_Store finds the
# background through an alias that includes it.
hdiutil_retry create -srcfolder "$STAGE" -volname "MailCraft" -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$DMG"
# The app inside the image must still pass --strict, so that regression can't ship again.
MOUNT="$WORK/dmg-check"
rm -rf "$MOUNT"
mkdir -p "$MOUNT"
hdiutil_retry attach -nobrowse -readonly -mountpoint "$MOUNT" "$DMG" >/dev/null
if ! codesign --verify --strict --deep --verbose=2 "$MOUNT/MailCraft.app"; then
  hdiutil_retry detach "$MOUNT" >/dev/null || true
  exit 1
fi
hdiutil_retry detach "$MOUNT" >/dev/null
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
if [ "$NOTARIZE" = 1 ]; then
  notarize "$DMG"
  stapler_retry "$DMG"
  spctl --assess --type open --context context:primary-signature -vvv "$DMG"
fi

# ---- CLI ---------------------------------------------------------------------------------------
echo "==> building $CLI_ZIP"
CLI_DIR="$WORK/mailcraft-cli-$VERSION-macos-$ARCH"
mkdir -p "$CLI_DIR"
cp "$WORK/bin/mailcraft-cli" "$CLI_DIR/"
copy_docs "$CLI_DIR"
sign --options runtime "$CLI_DIR/mailcraft-cli"
codesign --verify --strict --verbose=2 "$CLI_DIR/mailcraft-cli"
rm -f "$CLI_ZIP"
ditto -c -k --keepParent "$CLI_DIR" "$CLI_ZIP"
# A bare Mach-O can't carry a stapled ticket; Gatekeeper looks the notarization up online.
if [ "$NOTARIZE" = 1 ]; then notarize "$CLI_ZIP"; fi

"$WORK/bin/mailcraft-cli" --version
echo "==> done"
ls -lh "$DMG" "$CLI_ZIP"
