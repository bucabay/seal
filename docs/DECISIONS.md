# Design Decisions

Key choices and why, in the order they were made.

## Tauri over Electron

**Decision:** build the desktop app on Tauri v2 (Rust + webview) rather than
Electron.

**Rationale:**
- The Rust `keyring` crate exposes macOS Keychain, Linux Secret Service, and
  Windows Credential Manager behind one API — the exact primitive this app
  needs, natively.
- Bundle size: ~5–15 MB vs 150 MB+ for Electron, matching a small utility.
- Security: Rust eliminates entire classes of memory-safety bugs in the part of
  the code that touches credentials.
- Tauri v2 is production-ready with a first-class CLI plugin.

**Consequences:** the frontend runs in the OS webview (WebKitGTK / WebView2 /
WKWebView); packaging is per-platform via `tauri build`.

## `keyring` crate, then macOS shell-out to `security`

**Decision:** use the `keyring` crate for the keychain backend on Linux/Windows,
but on macOS shell out to the `security` CLI with `-A` (allow any app).

**Rationale (why not `keyring` everywhere):** `keyring`'s macOS backend calls
`SecKeychainAddGenericPassword` with a NULL access, which creates items with a
restrictive ACL. Because the CLI and GUI are different binaries (and dev vs
release builds differ again), macOS re-prompts — and, with an ad-hoc signed
app, sometimes fails with "not allowed" even after the user clicks Allow.

**Rationale (why `security -A`):** `security add-generic-password -A` creates
the item with a permissive ACL ("allow any application without warning"). The
item is still protected by the keychain itself (encrypted, locked with the
session); `-A` only removes the per-app re-prompt.

**Consequences:** macOS spawns a `security` process per operation (a few ms,
fine for user-initiated actions). A side benefit: the keychain-item ACL is
identical for the CLI and GUI, so cross-binary reads never prompt.

## Local index for listing

**Decision:** maintain `~/.config/seal/index.json` (keys only) instead of
listing from the keychain.

**Rationale:** keychain APIs cannot enumerate entries — `security
find-generic-password` returns a single match, and there is no cross-platform
"list all" call. A local index is the only way to power `seal list` and the
GUI's vault/secret lists.

**Consequences:** the index can go stale if secrets are added out-of-band (e.g.
another machine). It never holds secret values, so a stale index is a
correctness nit, not a security issue.

## CLI-only build via cargo feature gating

**Decision:** gate all Tauri/GUI code behind a `gui` cargo feature (default on)
and build the Homebrew formula with `--no-default-features`.

**Rationale:** the full Tauri build pulls in WebKit (C compilation via `cc`),
the Node/pnpm frontend toolchain, and on macOS an Xcode-version check — all
unnecessary for a CLI. This surfaced concretely: `brew install` failed with
"Your Xcode is outdated" and a pnpm build-script approval error.

**Consequences:** `cargo build --no-default-features` produces a ~600 KB CLI
with zero C compilation and no frontend dependency.

## Hand-rolled CLI argument parsing

**Decision:** parse arguments by hand rather than adding `clap`.

**Rationale:** four subcommands and one flag don't justify a heavyweight
dependency. Keeping the CLI dependency-free shrinks the binary and build time.

**Consequences:** the parser is a ~30-line loop in `main.rs`; fine at this
scale, would warrant `clap` if the surface grows.

## ShadCN/ui over GlueStack, Mantine, daisyUI, etc.

**Decision:** build the UI on ShadCN/ui (Radix primitives + Tailwind).

**Rationale:**
- **Radix primitives** are fully keyboard-accessible with correct ARIA and
  focus management — dialogs, dropdowns, and menus done right, for free.
- **Tailwind** tree-shakes to a few KB, matching Tauri's lightweight ethos.
- **Copy-paste ownership** — components live in `src/components/ui`, so the
  account/vault selector and dialogs can be styled to feel native, not
  "generic web".
- GlueStack is React Native-first (wrong tool for a desktop webview); Mantine
  and Ant are heavier with a less "native" feel.

## KiteDeploy-inspired design language

**Decision:** adopt kitedeploy.com's design language — Space Grotesk / DM Sans /
DM Mono, sharp corners, hairline borders, mono uppercase eyebrows, and an
orange-in-light / blue-in-dark brand.

**Rationale:** it fits a developer tool's technical aesthetic and gave a
cohesive, distinctive identity rather than an unbranded default theme.

**Consequences:** the exact tokens (colors, radius, fonts) are lifted into
`src/index.css` and `tailwind.config.cjs`; the theme is toggled via a `.dark`
class on `<html>`.

## Self-hosted fonts

**Decision:** bundle Space Grotesk, DM Sans, and DM Mono via `@fontsource`
instead of loading from Google Fonts.

**Rationale:** a desktop app must work offline and must not leak network
requests to a font CDN. `@fontsource` inlines the font files into the bundle.

## "Vault" terminology

**Decision:** call the secret-grouping dimension "vault", reserving "account"
for the (future) user login.

**Rationale:** the two concepts collided once login was introduced — "account"
meant both the login identity and the secret namespace. "Vault" is the
industry convention (1Password, Bitwarden) and fits the Seal metaphor.

**Consequences:** the rename touched UI strings, Tauri command names
(`list_vaults`, `add_vault`), the CLI (`--vault`, `SEAL_VAULT`), and docs. The
storage format was unchanged (`seal:{vault}:{key}`).

## Agent skill packaging

**Decision:** ship an agent skill (`skills/seal/SKILL.md`) that installs to
`~/.claude/skills/seal/` (read by both Claude Code and opencode), and encode
the safety contract for AI agents.

**Rationale:** AI agents are a primary consumer of a secrets CLI, and they need
explicit rules — never print a secret, never write secrets to files, consume
via `seal get` inline.

**Consequences:** the skill is installed by `install.sh`, shipped by the brew
formula to `share/seal/skills/seal/`, and documented in the README.
