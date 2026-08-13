# Architecture

## Overview

Seal is a single Rust package that compiles to two artifacts from the same
source: a **CLI** and a **Tauri GUI**. Both read and write secrets through a
shared keychain abstraction, and both maintain a small local index used for
listing (the keychain cannot enumerate).

```
┌─────────────────────────────────────────────────────────────┐
│                        Rust crate `seal`                    │
│                                                             │
│  ┌──────────────┐   ┌──────────────────┐                    │
│  │  main.rs     │   │  lib.rs          │                    │
│  │  (bin) CLI   │   │  (lib) Tauri GUI │                    │
│  │              │   │  commands        │                    │
│  └──────┬───────┘   └────────┬─────────┘                    │
│         │                    │                              │
│         └────────┬───────────┘                              │
│                  │                                          │
│       ┌──────────▼──────────┐        ┌──────────────────┐   │
│       │  keychain.rs        │        │  index.json      │   │
│       │  (platform backend) │        │  (~/.config/seal)│   │
│       └──────────┬──────────┘        └────────┬─────────┘   │
│                  │                            │             │
└──────────────────┼────────────────────────────┼─────────────┘
                   │                            │
        ┌──────────▼──────────┐        ┌─────────▼──────────┐
        │  OS keychain        │        │  keys only (no     │
        │  (secrets)          │        │  secret values)    │
        └─────────────────────┘        └────────────────────┘

Frontend (React + ShadCN) ── invoke() ──► lib.rs commands ──► keychain.rs
```

## Components

### `src-tauri/src/main.rs` — CLI (binary `seal`)

- Hand-rolled argument parser (no `clap` dependency).
- Commands: `set`/`save`, `get`, `delete`/`rm`, `list`/`ls`, `--help`.
- Flag `--vault`/`-v` and env `SEAL_VAULT` override the default vault.
- Key parsing: `ns/key` → vault `ns`, key `key`; bare `key` → default vault.
- No args: launches the GUI (when the `gui` feature is enabled) or prints usage.

### `src-tauri/src/lib.rs` — Tauri commands (library `seal_lib`)

Gated behind `#![cfg(feature = "gui")]` so the CLI-only build ships without
Tauri. Exposes six commands to the frontend via `invoke`:

| Command | Purpose |
|---|---|
| `save_secret(key, value, vault)` | store a secret |
| `get_secret(key, vault)` | retrieve a secret |
| `delete_secret(key, vault)` | remove a secret |
| `list_secrets(vault)` | list keys in a vault |
| `list_vaults()` | list all vault names |
| `add_vault(vault)` | create an empty vault |

### `src-tauri/src/keychain.rs` — platform backend

The single place that touches the OS keychain. Compiled into **both** crates.

- **macOS**: shells out to the `security` CLI, saving with `-A` (allow any app)
  so reads never prompt. See
  [DECISIONS.md](DECISIONS.md#macos-permissive-acl-via-security-cli).
- **Linux / Windows**: uses the `keyring` crate (`secret-service` /
  `windows-credentials`).

Public API (service is always `"seal"`):

```rust
keychain::set(account: &str, value: &str) -> Result<(), String>
keychain::get(account: &str)              -> Result<String, String>
keychain::delete(account: &str)           -> Result<(), String>
```

### Frontend (`src/`)

- `App.tsx` — top-level state (vaults, current vault, secrets, user session,
  dialog state) and the secret list.
- `components/header.tsx` — logo, vault selector, theme toggle, user menu.
- `components/vault-selector.tsx` — dropdown listing vaults + "Add vault".
- `components/user-menu.tsx` — avatar menu (sign in / sign out).
- `components/login-dialog.tsx`, `add-vault-dialog.tsx` — dialogs.
- `hooks/use-theme.tsx` — dark/light theme with localStorage persistence.
- `hooks/use-user.ts` — fake user session (login UI only; sync is stubbed).
- `components/ui/` — ShadCN primitives (button, dialog, dropdown-menu, avatar,
  input, label, separator).

## Storage model

### Keychain entries

Each secret is one generic-password entry:

- **service** = `seal` (constant, the app name)
- **account** = `"{vault}:{key}"` (e.g. `hardroad:db_pass`)
- **password** = the secret value

This maps to `security add-generic-password -s seal -a hardroad:db_pass -w …`
on macOS and `keyring::Entry::new("seal", "hardroad:db_pass")` on Linux/Windows.
The format is identical across platforms, so a vault's secrets are portable.

### Local index

Keychain APIs cannot enumerate, so keys are mirrored to a JSON file:

- macOS / Linux: `~/.config/seal/index.json`
- Windows: `%APPDATA%\seal\index.json`

Shape: `{ "vault_name": ["key1", "key2"] }` — **keys only, never values.**
If the index is missing or stale, `get`/`set`/`delete` still work; only `list`
(and the GUI's vault/secret list) relies on it.

## Feature gating

The `gui` cargo feature (default) pulls in Tauri. Building with
`--no-default-features` produces a lean CLI-only binary — no Tauri, no WebKit,
no C compilation, no frontend. This is what the Homebrew formula builds, which
sidesteps both the Xcode/WebKit toolchain and the Node/pnpm frontend build.

```
cargo build --manifest-path src-tauri/Cargo.toml                    # CLI + GUI
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features  # CLI only
```

## Distribution

- **Homebrew** (`brew/seal.rb`, tap `bucabay/homebrew-tap`) — builds CLI-only.
- **GitHub Releases** (`.github/workflows/release.yml`) — `tauri-action` builds
  and attaches the GUI bundles (`.dmg`, `.msi`/`.exe`, `.deb`/`.AppImage`) for
  macOS aarch64/x86_64, Windows, and Linux on every `v*` tag.
- **install.sh** — source installer: builds the CLI, symlinks it, installs the
  agent skill.
