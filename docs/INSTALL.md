# Install

Seal ships as a CLI and as a macOS/Windows/Linux desktop app. The CLI is a
Homebrew formula; the macOS app is a Homebrew cask. Neither requires Apple
code signing — the app is installed exactly as released.

## Homebrew — CLI (macOS, Linux)

```sh
brew install bucabay/tap/seal
seal --version
```

This builds the CLI from source, so it needs Rust (`brew` installs it if
missing). The tap also contains the desktop cask below.

## Homebrew — GUI app (macOS)

```sh
brew trust bucabay/tap                 # once, if Homebrew asks
brew install --cask bucabay/tap/seal
```

Homebrew installs the prebuilt `.dmg` from the latest GitHub release. The cask
does not sign or notarize the app, and macOS quarantines every download, so the
**first launch is blocked by Gatekeeper**. Clear the flag once:

```sh
xattr -dr com.apple.quarantine /Applications/Seal.app
```

Or try to open it, then allow Seal under **System Settings → Privacy &
Security → Open Anyway**. After that it launches normally.

The formula and the cask share the `seal` name:

| Command | Installs |
|---|---|
| `brew install seal` | CLI |
| `brew install --cask seal` | GUI app |

The cask is in this repository at [`brew/Casks/seal.rb`](../brew/Casks/seal.rb)
and is published in the [`bucabay/homebrew-tap`](https://github.com/bucabay/homebrew-tap)
tap.

## GitHub Releases — GUI app

Download the `.dmg` (macOS), `.msi`/`.exe` (Windows), or `.deb`/`.AppImage`
(Linux) from the [latest release](https://github.com/bucabay/seal/releases).

macOS builds are unsigned and not notarized. If Gatekeeper blocks the app, allow
it under System Settings → Privacy & Security, or run:

```sh
xattr -dr com.apple.quarantine /Applications/Seal.app
```

## From source

```sh
./install.sh        # builds and symlinks the `seal` CLI, installs the agent skill
pnpm tauri dev      # run the GUI in dev mode
```

`./install.sh` installs the skill for Claude Code, Codex, opencode and the
shared `~/.agents/skills` hub. To do only that step:

```sh
seal skill install
```

## Verify

```sh
seal set demo/example hello
seal list demo
```

## Signing

Unsigned builds use the standard OS keychain for unprotected items. Per-item
Touch ID requires a signed, provisioned macOS bundle with the shared
`keychain-access-groups` entitlement; see
[docs/BIOMETRICS.md](BIOMETRICS.md) for the current status and the signing
helper in `scripts/sign-macos.py`.
