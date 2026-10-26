#!/usr/bin/env bash
# Check the macOS release artifacts exactly as users download them:
#
#   $DIST/mailcraft-<version>-macos-<arch>.dmg       signed, notarized, ticket stapled; the app
#                                                   inside is Developer ID signed with the
#                                                   hardened runtime and carries every arch
#   $DIST/mailcraft-cli-<version>-macos-<arch>.zip   unpacked here; the binary inside must be
#                                                   Developer ID signed with the hardened runtime,
#                                                   timestamped and notarized
#
# Usage: packaging/macos/verify.sh [--arch universal|aarch64|x86_64]
#
# With a real signing identity and notarization credentials in the environment (the same
# MACOS_SIGN_IDENTITY / APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID that package.sh uses), every
# check is strict and a failure exits non-zero. Without them (local ad-hoc builds, or a release run
# whose secrets are missing) only the signatures' integrity is checked and the rest is a warning,
# matching package.sh. Nothing here reads or prints a secret's value; it only tests whether one is set.
#
# A bare Mach-O can't carry a stapled notarization ticket (only .app, .dmg and .pkg can), so for the
# CLI Gatekeeper looks the ticket up online the first time a quarantined copy runs. `spctl --assess
# --type install` does that same online lookup here, which is how this script proves the CLI was
# notarized.
set -euo pipefail
# shellcheck source=../common.sh
. "$(dirname "${BASH_SOURCE[0]}")/../common.sh"

APP_NAME=MailCraft

ARCH=universal
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    -h | --help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) WANT_ARCHS="x86_64 arm64" ;;
  aarch64) WANT_ARCHS="arm64" ;;
  x86_64) WANT_ARCHS="x86_64" ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

DMG="$DIST/mailcraft-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/mailcraft-cli-$VERSION-macos-$ARCH.zip"
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
STRICT=0
if [[ "$IDENTITY" != "-" ]] && [[ -n "${APPLE_ID:-}" ]] && [[ -n "${APPLE_PASSWORD:-}" ]] && [[ -n "${APPLE_TEAM_ID:-}" ]]; then
  STRICT=1
fi

FAILED=0
fail() {
  if [[ "$STRICT" = 1 ]]; then
    echo "error: $*" >&2
    FAILED=1
  else
    warn "$* (expected without signing secrets)"
  fi
}

# spctl's online ticket lookup can lag a few seconds behind notarytool's "Accepted".
assess() {
  local out attempt tries=1
  [[ "$STRICT" = 1 ]] && tries=5
  for attempt in $(seq "$tries"); do
    if out="$(spctl "$@" 2>&1)"; then
      printf '%s\n' "$out"
      return 0
    fi
    [[ "$attempt" = "$tries" ]] || sleep 10
  done
  printf '%s\n' "$out"
  return 1
}

# `stapler validate` asks Apple's CloudKit service too, which sometimes drops the connection
# (error 68) though the ticket is there.
validate_staple() {
  local target="$1" attempt
  for attempt in 1 2 3 4 5; do
    xcrun stapler validate "$target" && return 0
    [[ "$attempt" = 5 ]] || sleep $((attempt * 20))
  done
  return 1
}

# Fails unless binary $1 (named $2 in messages) carries every architecture of --arch.
check_archs() {
  local archs a
  archs="$(lipo -archs "$1")"
  for a in $WANT_ARCHS; do
    case " $archs " in
      *" $a "*) ;;
      *) echo "error: $2 lacks $a (has: $archs)" >&2; exit 1 ;;
    esac
  done
  return 0
}

# Developer ID, hardened runtime, secure timestamp and team of the signature on $1 (named $2).
check_identity() {
  local details
  details="$(codesign -d --verbose=2 "$1" 2>&1)"
  printf '%s\n' "$details" | grep -E '^(Identifier|Format|CodeDirectory|Authority|Timestamp|TeamIdentifier)[= ]' || true
  printf '%s\n' "$details" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' \
    || fail "$2 is not signed with the hardened runtime"
  printf '%s\n' "$details" | grep -q '^Authority=Developer ID Application:' \
    || fail "$2 is not signed with a Developer ID Application identity"
  printf '%s\n' "$details" | grep -q '^Timestamp=' \
    || fail "$2 signature has no secure timestamp"
  if [[ -n "${APPLE_TEAM_ID:-}" ]]; then
    printf '%s\n' "$details" | grep -qx "TeamIdentifier=$APPLE_TEAM_ID" \
      || fail "$2 is signed by a different team"
  fi
  return 0
}

for f in "$DMG" "$CLI_ZIP"; do
  if [[ ! -f "$f" ]]; then
    echo "error: missing $f" >&2
    exit 1
  fi
done

WORK="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/mailcraft-verify.XXXXXX")"
MNT="$WORK/mnt"
cleanup() {
  hdiutil detach "$MNT" -quiet >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> $(basename "$DMG")"
codesign --verify --strict --verbose=2 "$DMG"
if [[ "$STRICT" = 1 ]]; then
  validate_staple "$DMG" || fail "no notarization ticket stapled to the DMG"
  assess --assess --type open --context context:primary-signature -vv "$DMG" \
    || fail "Gatekeeper rejects the DMG"
fi

# ---- the app inside ----------------------------------------------------------------------------
mkdir -p "$MNT"
hdiutil attach "$DMG" -readonly -nobrowse -noautoopen -mountpoint "$MNT" -quiet
APP="$MNT/$APP_NAME.app"
EXE="$APP/Contents/MacOS/$APP_NAME"
if [[ ! -x "$EXE" ]]; then
  echo "error: $(basename "$DMG") has no $APP_NAME.app/Contents/MacOS/$APP_NAME" >&2
  exit 1
fi
echo "==> $APP_NAME.app"
check_archs "$EXE" "$APP_NAME"

# Integrity first: this must hold even for ad-hoc builds, and with --strict, on the app in the
# image and on a copy installed from it with ditto. A valid DMG signature says nothing about the
# bundle inside: the old makehybrid image gave every file a com.apple.FinderInfo, which strict
# mode rejects as "detritus".
codesign --verify --strict --deep --verbose=2 "$APP"
ditto "$APP" "$WORK/copy/$APP_NAME.app"
codesign --verify --strict --deep --verbose=2 "$WORK/copy/$APP_NAME.app"
check_identity "$APP" "$APP_NAME.app"
if [[ "$STRICT" = 1 ]]; then
  validate_staple "$APP" || fail "no notarization ticket stapled to $APP_NAME.app"
  if gk="$(assess --assess --type execute -vv "$APP")"; then
    printf '%s\n' "$gk"
    printf '%s\n' "$gk" | grep -q 'source=Notarized Developer ID' \
      || fail "Gatekeeper accepts $APP_NAME.app but not as notarized"
  else
    printf '%s\n' "$gk"
    fail "Gatekeeper rejects $APP_NAME.app (not notarized?)"
  fi
fi
hdiutil detach "$MNT" -quiet

# ---- CLI zip -----------------------------------------------------------------------------------
echo "==> $(basename "$CLI_ZIP")"
ditto -x -k "$CLI_ZIP" "$WORK/cli"
CLI="$WORK/cli/mailcraft-cli-$VERSION-macos-$ARCH/mailcraft-cli"
if [[ ! -x "$CLI" ]]; then
  echo "error: $(basename "$CLI_ZIP") has no executable mailcraft-cli-$VERSION-macos-$ARCH/mailcraft-cli" >&2
  exit 1
fi
check_archs "$CLI" mailcraft-cli
codesign --verify --strict --verbose=2 "$CLI"
check_identity "$CLI" mailcraft-cli
if gk="$(assess --assess --type install -vv "$CLI")"; then
  printf '%s\n' "$gk"
  printf '%s\n' "$gk" | grep -q 'source=Notarized Developer ID' \
    || fail "Gatekeeper accepts mailcraft-cli but not as notarized"
else
  printf '%s\n' "$gk"
  fail "Gatekeeper rejects mailcraft-cli (not notarized?)"
fi
"$CLI" --version

if [[ "$FAILED" != 0 ]]; then
  echo "error: macOS artifacts failed verification" >&2
  exit 1
fi

if [[ "$STRICT" = 1 ]]; then
  result="signed (Developer ID, hardened runtime), notarized; DMG and app stapled"
else
  result="signatures intact; Developer ID and notarization not required (no signing secrets)"
fi
echo "==> verified: $result"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  echo "- macOS \`$(basename "$DMG")\`, \`$(basename "$CLI_ZIP")\`: $result" >>"$GITHUB_STEP_SUMMARY"
fi
