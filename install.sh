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

# Install the agent skill (Claude Code + opencode both read ~/.claude/skills)
SKILL_DIR="${HOME}/.claude/skills/seal"
mkdir -p "$SKILL_DIR"
cp "$(pwd)/skills/seal/SKILL.md" "$SKILL_DIR/SKILL.md"
echo "==> Installed skill: $SKILL_DIR/SKILL.md"

echo "==> Verify with: seal --help"
