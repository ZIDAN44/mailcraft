# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../common.sh"`.
# (Not env.sh: .gitignore reserves packaging/env.sh for local, uncommitted settings.)
#
# Exports:
#   ROOT                    workspace root
#   VERSION                 [workspace.package] version from Cargo.toml (override: MAILCRAFT_VERSION,
#                           or the VERSION the shared release workflow sets for a run's override)
#   DIST                    output directory for release artifacts (default: $ROOT/dist/release)
#   MAILCRAFT_BUILD_SHA     git commit shown by `mailcraft --version` (default: BUILD_SHA, then git)
#   MAILCRAFT_BUILD_DATE    UTC build date, YYYY-MM-DD (default: BUILD_DATE, then today)
#   CARGO_TARGET_DIR        cargo's target dir (default: $ROOT/target)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in exactly one place: `[workspace.package] version` in the root Cargo.toml.
# (`cargo xtask version` prints the same thing; awk avoids compiling xtask here.)
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
}

VERSION="${MAILCRAFT_VERSION:-${VERSION:-$(workspace_version)}}"
if [ -z "$VERSION" ]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

# The shared release workflow (storytold/craft-common-actions) sets BUILD_SHA and BUILD_DATE.
MAILCRAFT_BUILD_SHA="${MAILCRAFT_BUILD_SHA:-${BUILD_SHA:-}}"
if [ -z "$MAILCRAFT_BUILD_SHA" ]; then
  MAILCRAFT_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
fi
export MAILCRAFT_BUILD_SHA
export MAILCRAFT_BUILD_DATE="${MAILCRAFT_BUILD_DATE:-${BUILD_DATE:-$(date -u +%Y-%m-%d)}}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

# Copy licence and readme files that exist into a package directory. Fonts built in from
# craft-fonts (CRAFT_FONTS_DIR) carry their licences inside the app (File › About).
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE ATTRIBUTION.md; do
    if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$dest/"; fi
  done
}

# Portable SHA-256 of a file (prints just the hash).
sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
