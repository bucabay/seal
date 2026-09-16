# Plan

What is built, what is next, in the order it should happen. Tick a box only
when it is covered by a passing test.

Run the suite with `cargo test --workspace`. Currently **133 passing**.

## Phase 1 — Jail the agent

Highest value per unit of effort, and nothing else is fully true without it: a
raw `security find-generic-password` walks around everything otherwise.

- [x] Seatbelt profile generation (shield and strict modes)
- [x] Deny the credential paths (keychain, `~/.ssh`, `~/.aws`, gcloud, kube, netrc, npmrc, pypirc, gnupg)
- [x] Deny *executing* `security`, `secret-tool`, `gpg`
- [x] Escape quotes and backslashes so a hostile path cannot inject a rule
- [x] `wrap_command` producing a runnable `sandbox-exec` argv
- [x] Test that the profile is actually accepted by `sandbox-exec` — text assertions are not enough
- [x] Test that the keychain directory is genuinely unreadable inside the jail
- [x] `keymaker jail -- <command>` and `keymaker jail --print`
- [ ] Linux enforcement: apply the `LandlockPlan` via Landlock + seccomp + a network namespace (currently generated, not applied)
- [ ] Windows: decide whether there is a mechanism worth using at all
- [ ] Metadata endpoint blocking on macOS — not expressible in sbpl; needs a packet filter or a network extension
- [ ] Measure how tight `strict` can be before real agent work breaks (a week of real use)

## Phase 2 — Broker

### 2a. Named tasks — the replacement for `get`

- [x] Manifest parsing: tasks, environments, inheritance, cycle rejection
- [x] Trust-on-first-use: propose → approve → promoted to a named task
- [x] Refuse to silently redefine an existing task
- [x] Runner injects values into the child's environment only
- [x] Unapproved commands refused before anything is read or spawned
- [x] Output redaction, with a leak flagged rather than silently patched
- [x] `keymaker run <task>` / `keymaker run -- <command>` / `list` / `doctor` / `set` / `rm`
- [ ] Scan files written during a run and fail loudly if a value landed in one

### 2b. Capability handles

- [x] Issue and redeem, one use by default, multi-use for retries
- [x] Bound to session, tool-call epoch, sequence, TTL, quota
- [x] A spent handle is indistinguishable from one that never existed
- [x] Ordering enforcement, opt-out for legitimate retries
- [x] Sweep expired handles
- [ ] Wire the registry into the daemon — currently exercised only by tests

### 2c. Constrained HTTP requests

- [x] Endpoint definitions: pinned method, host, path
- [x] Path templating that cannot walk to another endpoint
- [x] Header allowlist; the credential header cannot be supplied by the caller
- [x] CRLF rejection in header values
- [x] Body schema: required, optional, types, unknown-field rejection, size bound
- [x] Policy: allow / deny / step-up, evaluated before the secret is read
- [x] `prepare()` takes the value as an argument so the module cannot reach the store
- [ ] **Actually send the request** — everything up to the socket is done (`cmd_call` stops here)
- [ ] Redact the response before returning it
- [ ] Ship starter definitions: Anthropic, OpenAI, Stripe, GitHub, Cloudflare, Vercel

### 2d. The daemon

None of this is built. The library is shaped for it: the registry, policy and
catalog are all owned types with no global state.

- [ ] `keymaker serve` on a unix socket
- [ ] Peer credential auth (`SO_PEERCRED` / `LOCAL_PEERCRED`)
- [ ] Hold a `pidfd` on Linux so PID reuse cannot race the credential check
- [ ] Session lifecycle tied to the connection
- [ ] Thin client that carries a request and no key material
- [ ] MCP surface: `list`, `run`, `call`, `set`, `delete`, `request_approval` — and no read tool

## Phase 3 — Expiry

- [x] `Exchanger` trait, `Source`, minted-vs-static in the type
- [x] TTL capping, expiry checks, rejection of an already-dead credential
- [x] `require_ephemeral` refuses to fall back to a static value
- [ ] Real exchangers: AWS STS `AssumeRole`, GitHub App installation tokens, Stripe restricted keys
- [ ] RFC 8693 token exchange for anything with an STS
- [ ] Secure Enclave key wrapping the store, so unwrap needs user presence

## Phase 4 — Audit and approval

- [x] Hash-chained log, tamper detection, JSONL round trip
- [x] Test asserting no event variant can carry a value
- [ ] Persist the log and append on every decision (nothing writes to it yet)
- [ ] `keymaker audit --verify` against the real file
- [ ] Approval UI beyond a terminal prompt

## Phase 5 — Storage backends

- [x] In-memory (tests)
- [x] macOS Keychain
- [ ] Linux Secret Service
- [ ] Windows Credential Manager
- [ ] Decide whether items should be created with a restrictive ACL now that the jail exists — the jail closes the same hole far more cheaply, so this may never be worth it

## Phase 6 — GUI

The library is the contract: the CLI already goes through it and holds no logic
of its own, so the GUI is a second caller rather than a second implementation.

- [ ] Tauri shell, lifting seal's design language
- [ ] Reveal and copy — the **only** place a value can be read
- [ ] Task list and run, with output shown redacted
- [ ] Approval prompts for step-up policies
- [ ] Audit viewer

## Deliberately not planned

- `get`, `export`, `reveal`, or any other verb that returns a value to the CLI
- A `read` tool on the MCP surface
- Writing a `.env` file

Each of these is the one change that would give up the claim. `.env` *import*
is fine, and flows the safe direction.
