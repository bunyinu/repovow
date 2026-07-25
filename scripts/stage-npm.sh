#!/usr/bin/env bash
# Stage the Rust release binary into npm platform packages for local dev or CI.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TARGET="${REPOVOW_TARGET:-}"
NPM_PKG="${REPOVOW_NPM_PKG:-}"
SKIP_BUILD=0

detect_host() {
  local arch os
  arch="$(uname -m)"
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  case "${os}-${arch}" in
    linux-x86_64)  echo "x86_64-unknown-linux-gnu linux-x64-gnu" ;;
    linux-aarch64|linux-arm64) echo "aarch64-unknown-linux-gnu linux-arm64-gnu" ;;
    darwin-x86_64) echo "x86_64-apple-darwin darwin-x64" ;;
    darwin-arm64)  echo "aarch64-apple-darwin darwin-arm64" ;;
    *) echo "unsupported platform: ${os}-${arch}" >&2; exit 1 ;;
  esac
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) TARGET="$2"; shift 2 ;;
    --npm-pkg) NPM_PKG="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$TARGET" || -z "$NPM_PKG" ]]; then
  read -r TARGET NPM_PKG <<< "$(detect_host)"
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "Building repovow for ${TARGET}..."
  cargo build --release --target "$TARGET"
else
  echo "Staging prebuilt repovow for ${TARGET}..."
fi

BIN="target/${TARGET}/release/repovow"
if [[ ! -f "$BIN" ]]; then
  echo "binary not found at ${BIN}" >&2
  exit 1
fi

PLATFORM_DIR="npm/platforms/${NPM_PKG}"
mkdir -p "${PLATFORM_DIR}/bin"
cp "$BIN" "${PLATFORM_DIR}/bin/repovow"
chmod +x "${PLATFORM_DIR}/bin/repovow"

# Local dev: also vendor into repovow-cli for shim fallback
mkdir -p npm/repovow-cli/vendor
cp "$BIN" npm/repovow-cli/vendor/repovow
chmod +x npm/repovow-cli/vendor/repovow

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
node npm/repovow-cli/scripts/sync-version.js "$VERSION"

echo "Staged ${BIN} -> ${PLATFORM_DIR}/bin/repovow"
echo "Version: ${VERSION}"
