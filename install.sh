#!/usr/bin/env bash
# Install the Seal CLI binary to /usr/local/bin (or ~/.local/bin if no sudo)
set -euo pipefail

cd "$(dirname "$0")"

echo "==> Building Seal..."
pnpm install >/dev/null 2>&1 || true
pnpm build >/dev/null 2>&1 || true
cargo build --manifest-path src-tauri/Cargo.toml --release

BIN_SRC="$(pwd)/src-tauri/target/release/seal"

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

# Install the agent skill (Claude Code + opencode both read ~/.claude/skills)
SKILL_DIR="${HOME}/.claude/skills/seal"
mkdir -p "$SKILL_DIR"
cp "$(pwd)/skills/seal/SKILL.md" "$SKILL_DIR/SKILL.md"
echo "==> Installed skill: $SKILL_DIR/SKILL.md"

echo "==> Verify with: seal --help"
