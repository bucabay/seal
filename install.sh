#!/usr/bin/env bash
# Install the Seal CLI binary to /usr/local/bin (or ~/.local/bin if no sudo)
set -euo pipefail

cd "$(dirname "$0")"

if [ "$(uname -s)" = "Darwin" ] && [ -n "${SEAL_PROVISIONING_PROFILE:-}" ]; then
  : "${SEAL_SIGNING_IDENTITY:?Set SEAL_SIGNING_IDENTITY to your Apple code-signing identity}"
  : "${SEAL_TEAM_ID:?Set SEAL_TEAM_ID to your Apple signing team ID}"
fi

echo "==> Building Seal..."
pnpm install >/dev/null 2>&1 || true
pnpm build >/dev/null 2>&1 || true
cargo build --manifest-path src-tauri/Cargo.toml --release

BIN_SRC="$(pwd)/src-tauri/target/release/seal"

if [ "$(uname -s)" = "Darwin" ] && [ -n "${SEAL_PROVISIONING_PROFILE:-}" ]; then
  /usr/bin/python3 scripts/sign-macos.py \
    --identity "$SEAL_SIGNING_IDENTITY" \
    --team-id "$SEAL_TEAM_ID" \
    --profile "$SEAL_PROVISIONING_PROFILE" \
    "$BIN_SRC"
  BIN_SRC="$(pwd)/src-tauri/target/release/seal.app/Contents/MacOS/seal"
  # Verify launch before changing the installed CLI.
  /usr/bin/python3 -B -c 'import subprocess,sys; subprocess.run([sys.argv[1], "--version"], check=True, timeout=15)' "$BIN_SRC"
elif [ "$(uname -s)" = "Darwin" ] && [ -n "${SEAL_SIGNING_IDENTITY:-}" ]; then
  # Standard keychain access does not need restricted sharing entitlements.
  codesign --force --sign "$SEAL_SIGNING_IDENTITY" "$BIN_SRC"
fi

if [ -w /usr/local/bin ]; then
  INSTALL_DIR="/usr/local/bin"
elif [ -w "$HOME/.local/bin" ]; then
  INSTALL_DIR="$HOME/.local/bin"
else
  INSTALL_DIR="$HOME/.local/bin"
  mkdir -p "$INSTALL_DIR"
fi

ln -sf "$BIN_SRC" "$INSTALL_DIR/seal"
echo "==> Installed: $INSTALL_DIR/seal"

# A `seal` earlier in PATH (typically a Homebrew build) would silently shadow
# what we just installed, which looks exactly like the install having no effect.
hash -r 2>/dev/null || true
ACTIVE="$(command -v seal || true)"
if [ -n "$ACTIVE" ] && [ "$ACTIVE" != "$INSTALL_DIR/seal" ]; then
  echo "==> WARNING: '$ACTIVE' comes first in PATH and will be used instead."
  echo "    Remove it (e.g. 'brew uninstall seal') or put $INSTALL_DIR ahead of it."
fi

# Install the agent skill wherever Claude Code, Codex and opencode look for it.
# The skill is compiled into the binary, so this is the same on every platform.
echo "==> Installing agent skill..."
"$INSTALL_DIR/seal" skill install

echo "==> Verify with: seal --help"
