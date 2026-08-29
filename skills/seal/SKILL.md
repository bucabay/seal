---
name: seal
description: Store, retrieve, and manage secrets via the `seal` CLI (macOS Keychain / Linux Secret Service / Windows Credential Manager). Load when the user asks to save, fetch, or remove a secret, API key, token, password, or credential; when a task needs a credential that should not be hardcoded; or when the agent must avoid writing secrets to files, .env, or transcripts. Also load when refactoring code to read credentials from the OS keychain instead of env vars or source.
---

# Seal — secure key/value store

`seal` is a small CLI (plus Tauri GUI) that stores secrets in your operating
system's native keychain. No secrets file to protect, no sync to trust.

| OS | Backend |
|---|---|
| macOS | Keychain |
| Linux | Secret Service (GNOME Keyring / KDE Wallet) |
| Windows | Credential Manager |

## CLI reference

```sh
seal set <key> <value>        # save a secret (vault defaults to "seal")
seal set ns/key value         # save under vault "ns"
seal get <key>                # print a secret to stdout
seal get ns/key               # retrieve from vault "ns"
seal delete <key>             # delete a secret
seal list                     # list every key in every vault
seal list <pattern>           # filter keys (substring, or * / ? glob)

# Default vault via env or flag
SEAL_VAULT=gabe seal set api_key "..."
seal set -v gabe api_key "..."

# No args launches the GUI
seal
```

Keys with a `/` are namespaced: `hardroad/db_pass` means vault `hardroad`,
key `db_pass`. Keys without a `/` go to the default vault (`seal`, unless
`SEAL_VAULT` overrides it).

## Rules for agents (security)

1. **Never print a secret value to the transcript or terminal.** Use `seal get`
   directly in the command that consumes it, e.g.
   `export TOKEN="$(seal get github/token)"` — not `seal get github/token` alone
   followed by echoing.
2. **Never write secrets to files**, including `.env`, config files, logs, or
   code. `.env` is for non-secret defaults; the actual value comes from `seal`.
3. **Never commit secrets.** If a secret ends up in a file or the clipboard
   cache, scrub it and tell the user.
4. **Offer to store, don't store unasked.** When a task needs a key/token, ask
   the user for the value and store it once with `seal set`, then reference it
   by key thereafter.
5. **Namespace by vault.** Use `project/key` (e.g. `hardroad/db_pass`) so a
   single default keychain stays organized across repos.

## Common patterns

```sh
# Store once
seal set hardroad/stripe_sk "sk_live_..."

# Consume without exposing
sk="$(seal get hardroad/stripe_sk)"  # sk now in shell var only

# Inject into a process env
STRIPE_KEY="$(seal get hardroad/stripe_sk)" ./script.sh

# Check what's stored (keys only, never values)
seal list                 # everything, as vault/key
seal list hardroad        # just that project
seal list '*_token'       # quote globs so the shell doesn't expand them

# Rotate / update
seal set hardroad/stripe_sk "sk_live_new..."   # set overwrites
```

## Listing

Bare `seal list` prints every key in every vault. Keys in the default vault
(`seal`) print bare; namespaced keys print as `vault/key`, so any line can be
pasted straight into `seal get`.

An argument filters that list. It is a case-insensitive substring match against
both `vault/key` and the bare key, unless it contains `*` or `?`, in which case
it is matched as a glob (`*` spans `/`). Quote globs so the shell does not
expand them first. A pattern that matches nothing exits `1`.

```sh
seal list                 # all keys, all vaults
seal list rack            # substring -> racknerd-mailk/root, ...
seal list 'hardroad/*'    # one vault
seal list '*api*key'      # glob across vaults
```

`--vault`/`-v` or `SEAL_VAULT` scopes `list` to that one vault (and prints bare
keys), with any pattern still applied within it.

## Listing caveat

Keychain APIs cannot enumerate entries, so `seal list` reads a local index at
`~/.config/seal/index.json` (Linux/macOS) or `%APPDATA%\seal\index.json`
(Windows). That file holds **keys only** — no values. If it's missing or
stale, `seal get` still works; `list` just may not show keys saved from
another machine.
