# Seal

Cross-platform secrets manager. A small CLI + GUI backed by your operating
system's native keychain — no new secrets file to protect, no sync to trust.

| OS | Backend |
|---|---|
| macOS | Keychain |
| Linux | Secret Service (GNOME Keyring / KDE Wallet) |
| Windows | Credential Manager |

## Install

### Homebrew (macOS / Linux)

```sh
brew install bucabay/tap/seal
```

### From source

```sh
./install.sh        # builds and symlinks `seal` into your PATH
```

### Dev

```sh
pnpm install
pnpm tauri dev       # GUI
cargo run --manifest-path src-tauri/Cargo.toml -- set KEY VALUE   # CLI
```

## CLI

```sh
seal set <key> <value>      # save a secret (vault defaults to "seal")
seal set ns/key value       # save under vault "ns"
seal get <key>              # print a secret
seal delete <key>           # delete a secret
seal list [vault]           # list keys in a vault

SEAL_VAULT=gabe seal set api_key "..."   # change default vault
seal set -v gabe api_key "..."            # or via flag
```

No args launches the GUI.

## Agent skill

An agent skill ships in `skills/seal/` and is installed automatically by
`install.sh` to `~/.claude/skills/seal/` (read by both Claude Code and
opencode). The brew formula installs it to `share/seal/skills/seal/` and prints
the symlink command in its caveats.

The skill encodes the safety contract for agents: never print a secret value,
never write secrets to files, and consume via `seal get` inline
(`export TOKEN="$(seal get project/key)"`).

## Architecture

- **Rust core** (`src-tauri/src/`): `keyring` crate abstracts the three OS
  keychains behind one `Entry::new("seal", "vault:key")` API.
- **`main.rs`**: CLI parser (no deps — hand-rolled arg parsing).
- **`lib.rs`**: Tauri command handlers shared with the GUI.
- **Index**: keys are listed from `~/.config/seal/index.json` (or
  `%APPDATA%\seal\index.json`), because keychain APIs can't enumerate. Secrets
  themselves live only in the keychain.
