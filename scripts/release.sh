#!/usr/bin/env bash
# Local release helper: test, build, stage npm, optional global install.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

INSTALL_GLOBAL=0
SKIP_TESTS=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install-global) INSTALL_GLOBAL=1; shift ;;
    --skip-tests) SKIP_TESTS=1; shift ;;
    *) echo "usage: $0 [--install-global] [--skip-tests]" >&2; exit 1 ;;
  esac
done

if cargo fmt --version &>/dev/null; then
  echo "==> cargo fmt --check"
  cargo fmt --all -- --check
else
  echo "==> skip cargo fmt (rustfmt not installed)"
fi

if [[ "$SKIP_TESTS" -eq 0 ]]; then
  echo "==> cargo test"
  cargo test --all-targets
fi

if cargo clippy --version &>/dev/null; then
  echo "==> cargo clippy"
  cargo clippy --all-targets -- -D warnings
else
  echo "==> skip cargo clippy (not installed)"
fi

echo "==> stage npm"
chmod +x scripts/stage-npm.sh
./scripts/stage-npm.sh

echo "==> verify npm shim"
node npm/repovow-cli/scripts/verify-shim.js

if [[ "$INSTALL_GLOBAL" -eq 1 ]]; then
  # Remove only the superseded packages and shim owned by this project.
  npm uninstall -g @keel2026/cli @keel-agent/cli >/dev/null 2>&1 || true
  if [[ -f "${HOME}/.local/bin/keel" ]] && "${HOME}/.local/bin/keel" --version 2>/dev/null | grep -Eq '^keel [0-9]'; then
    echo "==> removing superseded ~/.local/bin/keel"
    rm -f "${HOME}/.local/bin/keel"
  fi
  echo "==> npm install -g ./npm/repovow-cli"
  npm install -g ./npm/repovow-cli
  NPM_BIN="$(npm prefix -g)/bin"
  if [[ -x "${NPM_BIN}/repovow" && "${NPM_BIN}/repovow" != "${HOME}/.local/bin/repovow" ]]; then
    mkdir -p "${HOME}/.local/bin"
    ln -sf "${NPM_BIN}/repovow" "${HOME}/.local/bin/repovow"
  fi
  echo "Installed: $(command -v repovow)"
  repovow --version
fi

echo ""
echo "Done. Next:"
echo "  npm install -g ./npm/repovow-cli    # global install"
echo "  repovow init                        # in your repo"
echo "  cargo install --path .           # alternative to npm"
