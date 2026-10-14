#!/usr/bin/env bash
# Regenerate every app icon in assets/app-icon/ from its definition in xtask/src/icons.rs:
#
#   assets/app-icon/hicolor/*/apps/ai.storyteller.mailcraft.{png,svg}   Linux, FreeBSD (16–512 px + SVG)
#   assets/app-icon/mailcraft-1024.png                                  the master
#   assets/app-icon/mailcraft.ico                                       Windows (16–256 px)
#   assets/app-icon/mailcraft.icns                                      macOS (16–1024 px)
#
# Pure Rust, so it runs anywhere (no sips/iconutil/ImageMagick). The outputs are committed, so
# builds and packaging never need this.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
(cd "$ROOT" && cargo run -q -p xtask -- icons)
