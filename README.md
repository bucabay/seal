# Seal

Cross-platform secrets manager. A small CLI + GUI backed by your operating
system's native keychain — no secrets file to protect, no sync to trust.

| OS | Backend |
|---|---|
| macOS | Keychain |
| Linux | Secret Service (GNOME Keyring / KDE Wallet) |
| Windows | Credential Manager |

## Features

- **CLI** — `seal set/get/delete/list` for scripting and terminal use
- **GUI** — Tauri app (macOS / Windows / Linux) with a vault selector, secret
  list, reveal/copy/delete, dark & light themes
- **Vaults** — group secrets by project (`hardroad/db_pass` = vault `hardroad`,
  key `db_pass`)
- **Zero-trust storage** — secrets live only in the OS keychain; nothing
  sensitive is written to disk
- **Agent skill** — ships with a Claude Code / opencode skill that encodes the
  safety contract for AI agents (never print secrets, never write them to files)

## Install

### Homebrew (macOS / Linux) — CLI

```sh
brew install bucabay/tap/seal
```

### GitHub Releases — GUI app

Download the `.dmg` (macOS), `.msi`/`.exe` (Windows), or `.deb`/`.AppImage`
(Linux) from the [latest release](https://github.com/bucabay/seal/releases).

> macOS builds are unsigned — if Gatekeeper blocks the app, right-click → Open,
> or run `xattr -dr com.apple.quarantine /Applications/Seal.app`.

### From source

```sh
./install.sh        # builds and symlinks the `seal` CLI + installs the agent skill
pnpm tauri dev      # run the GUI in dev mode
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

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — components, storage model, data flow
- [Design decisions](docs/DECISIONS.md) — rationale for every major choice
- [Design system](docs/DESIGN.md) — UI framework, theming, typography

## Development

```sh
pnpm install
pnpm tauri dev                                  # GUI
cargo run --manifest-path src-tauri/Cargo.toml -- set KEY VALUE   # CLI
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features   # CLI-only (no Tauri)
pnpm build                                      # type-check + build frontend
```

Releases are built by [`.github/workflows/release.yml`](.github/workflows/release.yml)
on every `v*` tag, producing installers for all three platforms.

## License

MIT — see [LICENSE](LICENSE).
