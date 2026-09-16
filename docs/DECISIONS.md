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

## Local index for listing — as a cache, not a source of truth

**Decision:** maintain an index of key names (never values) at
`~/Library/Application Support/seal/index.json` (macOS),
`~/.config/seal/index.json` (Linux) or `%APPDATA%\seal\index.json` (Windows),
**but** re-derive it from the keychain on every `list` wherever the platform can
enumerate.

**Rationale:** the keyed APIs Seal writes through cannot enumerate — `security
find-generic-password` returns a single match, and `keyring::Entry` is lookup
only — so something has to remember the names. Treating that file as the *truth*
was the mistake: it is absent on a fresh machine, absent after a keychain
restore, and wrong whenever an entry is written by another build or removed with
`security` directly. In each case `seal list` printed nothing while the secrets
were sitting in the keychain, which reads as "Seal lost my data".

macOS can in fact enumerate, via `security dump-keychain`. That prints item
*attributes* — service and account names — and nothing else; a value requires
`-d`, which Seal never passes. So on macOS `list` reads the names back from the
keychain, rewrites the index from them, and needs no repair step.

**Consequences:** on macOS the index cannot drift — out-of-band writes appear on
the next `list`, and out-of-band deletions disappear. An empty enumeration
(locked or unreadable keychain) is treated as "no information" and leaves a
populated index intact, so a transient failure cannot erase it. On Linux and
Windows the index remains the only source and can still go stale; it never holds
secret values, so that stays a correctness nit rather than a security issue.

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

## No read verb — Seal as ssh-agent for secrets

**Decision:** remove `seal get`, never build `seal export`, and make `seal run`
the only way to consume a secret. The CLI cannot emit a value. Humans read
values in the GUI; agents never do.

**Rationale:** Seal's primary user is a coding agent, and the property worth
selling is that a value never enters the model's context. Today that is enforced
by a *rule* in the agent skill ("never print a secret value"), which a confused,
injected, or sloppy model ignores — silently and irreversibly, since a key in a
transcript cannot be recalled. A capability that does not exist cannot be
misused. `ssh-agent` is the precedent: it signs, it does not hand over the key,
and the absence of an export verb is the security property rather than a gap.

**Consequences:** `seal run` must ship before `get` is removed, or the tool has
no consuming verb. Every script and skill using `export TOKEN="$(seal get …)"`
must migrate. There is no bulk read, so `export` and "write a `.env` file" are
out — both are `get` in a larger wrapper; `.env` *import* stays, as it flows the
safe direction. The GUI becomes load-bearing as the human surface and therefore
has to be signed. The claim stays narrow until the permissive keychain ACL is
fixed, since `security find-generic-password` currently walks around Seal
entirely — see [AGENT-MODEL.md](AGENT-MODEL.md).
