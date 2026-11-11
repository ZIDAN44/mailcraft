#!/usr/bin/env bash
# Build and package MailCraft for FreeBSD:
#
#   $DIST/mailcraft-<version>-freebsd-<arch>.tar.gz   bin/, share/ (desktop entry, icons, AppStream)
#
# Usage: packaging/freebsd/package.sh [--skip-build]
#
# Run on FreeBSD (CI: a FreeBSD 15 VM via vmactions/freebsd-vm). Runtime needs: libxkbcommon,
# wayland or libX11 and mesa (Vulkan or EGL): `pkg install libxkbcommon libX11 mesa-libs vulkan-loader`;
# a Secret Service provider (gnome-keyring, KeePassXC) keeps the passwords.
set -euo pipefail
# shellcheck source=../common.sh
. "$(dirname "${BASH_SOURCE[0]}")/../common.sh"
LINUX="$ROOT/packaging/linux"
APP_ID=io.github.zidan44.MailCraft

# The release workflow cross-builds aarch64 in the x86_64 VM with CARGO_BUILD_TARGET: then the
# archive is named after the target, not the machine building it.
if [ -n "${CARGO_BUILD_TARGET:-}" ]; then
  case "$CARGO_BUILD_TARGET" in
    aarch64-unknown-freebsd) ARCH=aarch64 ;;
    x86_64-unknown-freebsd) ARCH=x86_64 ;;
    *) echo "unsupported CARGO_BUILD_TARGET $CARGO_BUILD_TARGET (expected x86_64-unknown-freebsd or aarch64-unknown-freebsd)" >&2; exit 2 ;;
  esac
else
  ARCH="$(uname -m)"
  case "$ARCH" in
    amd64 | x86_64) ARCH=x86_64 ;;
    arm64 | aarch64) ARCH=aarch64 ;;
    *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
  esac
fi

if [ "${1:-}" != "--skip-build" ]; then
  (cd "$ROOT" && cargo build --release --locked -p mailcraft -p mailcraft-cli ${CARGO_BUILD_TARGET:+--target "$CARGO_BUILD_TARGET"})
fi
BIN="$CARGO_TARGET_DIR/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release"

# Refuse a missing, wrong-arch or non-FreeBSD binary (also with --skip-build), which would ship
# under this arch's name. Bytes: ELF magic, 64-bit little-endian, OSABI SYSV or FreeBSD,
# e_machine. FreeBSD's od pads its columns unlike GNU's, so only the hex digits are compared (as
# DeckCraft's script).
case "$ARCH" in
  x86_64) MACHINE=3e00 ;;
  aarch64) MACHINE=b700 ;;
esac
hex() { od -An -tx1 "$@" | tr -d ' \n'; }
for f in "$BIN/mailcraft" "$BIN/mailcraft-cli"; do
  [ -f "$f" ] || { echo "error: $f was not built" >&2; exit 1; }
  [ "$(hex -N6 "$f")" = 7f454c460201 ] || { echo "error: $f is not a 64-bit little-endian ELF (starts $(hex -N6 "$f"))" >&2; exit 1; }
  case "$(hex -j7 -N1 "$f")" in
    00 | 09) ;;
    *) echo "error: $f is not a FreeBSD ELF (OSABI $(hex -j7 -N1 "$f"))" >&2; exit 1 ;;
  esac
  [ "$(hex -j18 -N2 "$f")" = "$MACHINE" ] || { echo "error: $f is not an ELF for $ARCH (e_machine $(hex -j18 -N2 "$f"))" >&2; exit 1; }
done
NAME="mailcraft-$VERSION-freebsd-$ARCH"
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$NAME"
rm -rf "$WORK"
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/metainfo" "$STAGE/share/icons" "$STAGE/share/doc/mailcraft"

install -m 755 "$BIN/mailcraft" "$BIN/mailcraft-cli" "$STAGE/bin/"
strip "$STAGE/bin/mailcraft" "$STAGE/bin/mailcraft-cli" 2>/dev/null || true
install -m 644 "$LINUX/$APP_ID.desktop" "$STAGE/share/applications/"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$MAILCRAFT_BUILD_DATE/g" \
  "$LINUX/$APP_ID.metainfo.xml.in" >"$STAGE/share/metainfo/$APP_ID.metainfo.xml"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/share/icons/"
copy_docs "$STAGE/share/doc/mailcraft"

tar -C "$WORK" -czf "$DIST/$NAME.tar.gz" "$NAME"
# A cross-built binary can't run here.
if [ -z "${CARGO_BUILD_TARGET:-}" ] || [ "$ARCH" = "$(uname -m | sed -e s/amd64/x86_64/ -e s/arm64/aarch64/)" ]; then
  "$STAGE/bin/mailcraft-cli" --version
fi
echo "wrote $DIST/$NAME.tar.gz"
ls -lh "$DIST/$NAME.tar.gz"
