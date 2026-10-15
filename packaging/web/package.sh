#!/usr/bin/env bash
# Build the browser version and zip it:  $DIST/mailcraft-web-<version>.zip
#
# Usage: packaging/web/package.sh [--skip-build]
#
# Needs: trunk (cargo install trunk --locked) and the wasm32-unknown-unknown target. The zip holds
# a self-contained static site in mailcraft-web-<version>/ that works from any URL path and inside
# an <iframe>. Hosting notes: packaging/web/README.md.
#
# Fonts: with CRAFT_FONTS_DIR set (a storytold/craft-fonts checkout), the Arabic, Japanese and Thai
# fallback fonts are built in. The shared release workflow doesn't check craft-fonts out for the
# web job, so in CI (GITHUB_ACTIONS) this fetches it at CRAFT_FONTS_REF, the commit release.yml
# pins as craft-fonts-ref (keep the two the same).
set -euo pipefail
# shellcheck source=../common.sh
. "$(dirname "${BASH_SOURCE[0]}")/../common.sh"
HERE="$ROOT/packaging/web"

CRAFT_FONTS_REF="${CRAFT_FONTS_REF:-3d9ab3a132ed0eff457e424226a435a901da30d3}"
if [ "${1:-}" != "--skip-build" ]; then
  command -v trunk >/dev/null || { echo "error: trunk not found (cargo install trunk --locked)" >&2; exit 1; }
  if [ -z "${CRAFT_FONTS_DIR:-}" ] && [ "${GITHUB_ACTIONS:-}" = "true" ]; then
    # Outside the target folder: the build cache's cleanup walks that and trips over a checkout.
    FONTS="${RUNNER_TEMP:-$CARGO_TARGET_DIR}/craft-fonts"
    rm -rf "$FONTS"
    git init -q "$FONTS"
    git -C "$FONTS" fetch -q --depth 1 https://github.com/storytold/craft-fonts.git "$CRAFT_FONTS_REF"
    git -C "$FONTS" checkout -q FETCH_HEAD
    export CRAFT_FONTS_DIR="$FONTS" CRAFT_FONTS_REQUIRED=1
  fi
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
# Sample server configs (MIME type, caching, compression); harmless where unused.
cp "$HERE/_headers" "$HERE/.htaccess" "$WORK/$NAME/"
cp "$HERE/README.md" "$WORK/$NAME/HOSTING.md"
copy_docs "$WORK/$NAME"
rm -f "$DIST/$NAME.zip"
(cd "$WORK" && zip -qr9 "$DIST/$NAME.zip" "$NAME")
echo "wrote $DIST/$NAME.zip"
ls -lh "$DIST/$NAME.zip"
