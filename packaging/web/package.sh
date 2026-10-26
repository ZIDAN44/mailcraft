#!/usr/bin/env bash
# Build the browser version and zip it:  $DIST/mailcraft-web-<version>.zip
#
# Usage: packaging/web/package.sh [--skip-build]
#
# Needs: trunk (cargo install trunk --locked) and the wasm32-unknown-unknown target. The zip holds
# a self-contained static site in mailcraft-web-<version>/ that works from any URL path and inside
# an <iframe>. Hosting notes: packaging/web/README.md.
#
# Fonts: with CRAFT_FONTS_DIR set (an absolute path to a storytold/craft-fonts checkout), the
# browser's fallback fonts are built in (crates/ui-egui/build.rs picks which); without it, none
# are. The release workflows check craft-fonts out at their pin and set it.
set -euo pipefail
# shellcheck source=../common.sh
. "$(dirname "${BASH_SOURCE[0]}")/../common.sh"
HERE="$ROOT/packaging/web"

if [ "${1:-}" != "--skip-build" ]; then
  command -v trunk >/dev/null || { echo "error: trunk not found (cargo install trunk --locked)" >&2; exit 1; }
  (cd "$ROOT/apps/mailcraft-web" && trunk build --release)
fi

SITE="$ROOT/dist/web"
[ -f "$SITE/index.html" ] || { echo "error: $SITE/index.html missing; run without --skip-build" >&2; exit 1; }
# Paths must be relative so the site works under any prefix (public_url = "./" in Trunk.toml).
if grep -Eq '(src|href)="/[^/]' "$SITE/index.html"; then
  echo "error: $SITE/index.html has root-absolute URLs; it would break when served from a sub-path" >&2
  exit 1
fi

NAME="mailcraft-web-$VERSION"
WORK="$CARGO_TARGET_DIR/web-package"
rm -rf "$WORK"
mkdir -p "$WORK/$NAME"
cp -R "$SITE/." "$WORK/$NAME/"
# Simplified Chinese: Noto Sans CJK SC is too large to build into the page (16 MB), so it goes
# beside it and the page fetches it only when it's wanted (the interface in Simplified Chinese,
# or mail with characters no other font has). Checked against craft-fonts' manifest first.
if [ -n "${CRAFT_FONTS_DIR:-}" ]; then
  row=$(grep '^Noto Sans CJK SC | Regular |' "$CRAFT_FONTS_DIR/fonts/manifest.txt" || true)
  if [ -n "$row" ]; then
    file=$(echo "$row" | awk -F' [|] ' '{print $3}')
    licence=$(echo "$row" | awk -F' [|] ' '{print $6}')
    want=$(echo "$row" | awk -F' [|] ' '{print $7}')
    got=$(sha256sum "$CRAFT_FONTS_DIR/$file" | cut -d' ' -f1)
    [ "$got" = "$want" ] || { echo "error: $file: SHA-256 $got, the manifest says $want" >&2; exit 1; }
    mkdir -p "$WORK/$NAME/fonts"
    cp "$CRAFT_FONTS_DIR/$file" "$WORK/$NAME/fonts/noto-sans-cjk-sc.otf"
    cp "$CRAFT_FONTS_DIR/$licence" "$WORK/$NAME/fonts/noto-sans-cjk-sc-OFL.txt"
  fi
fi
# Sample server configs (MIME type, caching, compression); harmless where unused.
cp "$HERE/_headers" "$HERE/.htaccess" "$WORK/$NAME/"
cp "$HERE/README.md" "$WORK/$NAME/HOSTING.md"
copy_docs "$WORK/$NAME"
rm -f "$DIST/$NAME.zip"
(cd "$WORK" && zip -qr9 "$DIST/$NAME.zip" "$NAME")
echo "wrote $DIST/$NAME.zip"
ls -lh "$DIST/$NAME.zip"
