# Plan

What is built, what is next, in the order it should happen. Tick a box only
when it is covered by a passing test.

Run the suite with `cargo test --workspace`. Currently **203 passing**.

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
- [x] Wired into the broker: a handle names what it authorises, so a handle for
      one task cannot run another, and one for a request cannot run a task

### 2c. Constrained HTTP requests

- [x] Endpoint definitions: pinned method, host, path
- [x] Path templating that cannot walk to another endpoint
- [x] Header allowlist; the credential header cannot be supplied by the caller
- [x] CRLF rejection in header values
- [x] Body schema: required, optional, types, unknown-field rejection, size bound
- [x] Policy: allow / deny / step-up, evaluated before the secret is read
- [x] `prepare()` takes the value as an argument so the module cannot reach the store
- [x] Send the request — https only, and redirects refused, because a redirect
      would carry the credential to a host the definition never pinned
- [x] Redact the response before returning it, and report when it echoed
- [x] Both body encodings: JSON and form. Supporting only one would leave half
      the useful APIs undefinable — Stripe takes form bodies, most modern APIs
      take JSON
- [x] Fixed headers a definition always sets (an API version, an `Accept`), which
      a caller can neither supply nor override
- [x] A `json` field type for genuinely open shapes such as a `messages` array.
      Everything else about the request stays pinned; only that field's shape
      goes unchecked
- [x] Starter definitions in `endpoints/`: Anthropic, OpenAI, Stripe, GitHub,
      Cloudflare — 11 endpoints, every one validated by a test that checks it
      pins a host, never carries a credential, and never lets a caller supply
      the header the credential goes in
- [ ] More services. This is where contributions land, and each one is a
      security control rather than glue

### 2d. The daemon

- [x] `keymaker serve` on a unix socket, 0700 directory and 0600 socket
- [x] Peer credential auth (`SO_PEERCRED` on Linux, `getpeereid` +
      `LOCAL_PEERPID` on macOS) — identity from the kernel, never self-reported
- [x] PID reuse closed by comparing the peer's process *start time*, not just
      its number. This works on both platforms; a Linux `pidfd` would be
      tidier and is still worth doing, but the race is already shut
- [x] Session lifecycle tied to the connection: closing the socket destroys
      every handle it earned
- [x] Thin client that carries a request and no key material
- [x] Wire format with no operation that returns a value, asserted by a test
      that fails if one is ever added
- [x] Socket ownership via `flock`, not a `connect()` probe — a listener that
      has just closed can still accept for a moment, so "is anyone there?" is
      a race and the lock is not
- [ ] MCP surface over the same dispatch: `list`, `run`, `call`, `set`,
      `delete`, `request_approval` — and no read tool

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
- [x] The broker appends on every session, grant, redemption, policy decision,
      task run and request — recording a handle *prefix*, never a redeemable one
- [x] Step-up approval: a call that needs a human is refused until one says
      yes, and the approval authorises exactly one call
- [x] Persist to disk, appended before an entry is acknowledged — an entry that
      is not written is not evidence
- [x] Refuse to start against a log whose chain is already broken, rather than
      appending and burying the break
- [x] `keymaker audit` and `keymaker audit --verify` against the real file
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
