# Seal

Cross-platform secrets manager. A small CLI + GUI backed by your operating
system's native keychain — no secrets file to protect, no sync to trust.

![The Seal GUI: vault and environment selectors above a list of masked secrets,
with reveal, copy and delete on each row](docs/images/seal-gui.png)

| OS | Backend |
|---|---|
| macOS | Keychain |
| Linux | Secret Service (GNOME Keyring / KDE Wallet) |
| Windows | Credential Manager |

## Features

- **CLI** — `seal set/get/delete/list` for scripting and terminal use
- **GUI** — Tauri app (macOS / Windows / Linux) with vault and environment
  selectors, a secret list with inline editing that saves itself, reveal, copy,
  delete, and dark & light themes
- **Vaults** — group secrets by project (`hardroad/db_pass` = vault `hardroad`,
  key `db_pass`)
- **Environments** — per-vault overlays (`dev`, `staging`, `production`) that
  inherit from what they extend, so each one stores only what it overrides
- **Zero-trust storage** — secrets live only in the OS keychain; nothing
  sensitive is written to disk
- **Per-item Touch ID (macOS)** — signed builds can protect individual items
  with Touch ID or password fallback; list/health checks never read passwords
- **Agent skill** — one Agent Skills `SKILL.md` for Claude Code, Codex,
  opencode and any tool that reads the format; encodes the safety contract for
  AI agents (never print a secret, never write one to a file)

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
seal protect <key>          # upgrade a legacy item (provisioned macOS build)
seal list                   # list every key in every vault
seal list <ns>              # list one vault: seal list hardroad
seal list <pattern>         # filter: substring, or glob with * and ?

seal env                    # list environments and what each extends
seal env add <name>         # declare one (extends `default` unless --extends)
seal env rm <name>          # remove an empty environment

SEAL_VAULT=gabe seal set api_key "..."   # change default vault
seal set -v gabe api_key "..."            # or via flag
SEAL_ENV=production seal get db_url       # change environment
seal get db_url -e production             # or via flag
```

![Seal from the terminal: saving two secrets, reading one back, and listing
everything in a vault](docs/images/seal-cli.gif)

No args launches the GUI.

## Environments

Every vault has its own environments, rooted at `default`. An environment
stores only the keys it overrides; anything else is read from the environment
it extends, so `production` needs a `db_url` but not a copy of every other
secret:

```sh
seal env add production -v mailkite            # extends default
seal env add dev --extends production -v mailkite
seal set mailkite/db_url "postgres://..." -e production
seal get mailkite/api_key -e dev               # dev -> production -> default
seal list -v mailkite -e dev                   # effective config, inherited marked
```

Reads walk that chain nearest-first. Deletes do not: `seal delete` only removes
what the named environment owns, and tells you where an inherited value
actually lives rather than pulling it out from under its siblings.

Secrets written before environments existed are unaffected — the `default`
environment uses the same keychain addressing it always did, so nothing needs
migrating.

## Agent skill

Seal ships an [Agent Skill](skills/seal/SKILL.md) that encodes the safety
contract for coding agents: never print a secret value, never write secrets to
files, and consume via `seal get` inline
(`export TOKEN="$(seal get project/key)"`).

`seal skill install` writes it wherever each agent looks — Claude Code, Codex,
opencode, and the shared `~/.agents/skills` hub — using the same layout on
macOS, Linux, and Windows:

```sh
seal skill install
seal skill install --dir <root>     # any other skills root
```

Only agents already installed on the machine get a directory (plus the shared
hub), so a fresh home directory is not littered with empty agent folders.
`./install.sh` runs it automatically, and the brew formula prints the command
in its caveats.

## Documentation

- [Agent model](docs/AGENT-MODEL.md) — why there is no read verb; ssh-agent for secrets
- [Plan D](docs/PLAN-D.md) — the three ways to use a secret without seeing it
- [keymaker/](keymaker/) — the agent-only implementation of Plan D (separate CLI + library)
- [Architecture](docs/ARCHITECTURE.md) — components, storage model, data flow
- [Biometric access](docs/BIOMETRICS.md) — per-item Touch ID, upgrades, and signing
- [Design decisions](docs/DECISIONS.md) — rationale for every major choice
- [Design system](docs/DESIGN.md) — UI framework, theming, typography

## Development

```sh
pnpm install
pnpm tauri dev                                  # GUI
cargo run --manifest-path src-tauri/Cargo.toml -- set KEY VALUE   # CLI
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features   # CLI-only (no Tauri)
pnpm build                                      # type-check + build frontend
vhs scripts/demo.tape                           # re-record the CLI demo GIF
```

Releases are built by [`.github/workflows/release.yml`](.github/workflows/release.yml)
on every `v*` tag, producing installers for all three platforms.

## License

MIT — see [LICENSE](LICENSE).
