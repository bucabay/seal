# Plan

What is built, what is next, in the order it should happen. Tick a box only
when it is covered by a passing test.

Run the suite with `cargo test --workspace`. Currently **309 passing** (307 in the workspace, 2 in the GUI crate).

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
- [x] Linux enforcement via Landlock. Landlock only grants, so the deny-list is
      inverted into the equivalent grant-list; that computation is tested on
      every platform and only the syscalls are Linux-gated
- [x] `Profile::enforcement()` reports which mechanism is available, and `jail`
      refuses to run where there is none rather than implying protection
- [x] Windows: no jail. There is no equivalent of Landlock or seatbelt that is
      worth the complexity here, so `enforcement()` returns `None` and the
      command refuses rather than pretending
- [x] **Decided: not closed, and said so.** Metadata endpoint blocking is not
      expressible in sbpl, and Landlock has no network rules. A namespace with a
      packet filter would also cut off the network the agent legitimately needs.
      `Profile::blocks_hosts()` returns false on macOS so no caller can assume
      otherwise, and the gap is documented rather than papered over
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
- [x] Scan files written during a run and report any that contain a value —
      bounded by depth, count and size, skipping build and VCS directories, and
      refusing to follow symlinks. An incomplete search says so rather than
      looking clean

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
- [x] MCP surface over the same dispatch: `list`, `run`, `call`, `next_turn`,
      `request_approval` — and no read tool. `TOOLS` is a fixed list and a test
      enumerates it, so adding anything value-shaped fails the build
- [x] `initialize` tells the model the rule rather than leaving it to infer one
- [x] `request_approval` reaches no dispatcher at all: an agent must not be able
      to approve its own request

## Phase 3 — Expiry

- [x] `Exchanger` trait, `Source`, minted-vs-static in the type
- [x] TTL capping, expiry checks, rejection of an already-dead credential
- [x] `require_ephemeral` refuses to fall back to a static value
- [x] RFC 8693 token exchange — the generic mechanism, working with any STS.
      Carries `actor_token`, so "who authorised this" stays answerable. A
      missing `expires_in` falls back to the requested TTL: unknown is not the
      same as long
- [x] AWS STS `AssumeRole`, with a SigV4 implementation checked against AWS's
      published `get-vanilla` vector and independently re-derived in Python
- [x] Multi-part credentials: AWS returns three secrets and all three are
      offered to the redactor, rather than only the session token
- [ ] GitHub App installation tokens (needs RS256, so an RSA dependency)
- [ ] Stripe has no minting API — restricted keys are created by hand, so there
      is nothing to automate and this should not be faked
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
- [x] `keymaker audit --verify` against the real file
- [x] Approval beyond a terminal prompt: the broker and the GUI share a queue
      on disk, so a person can answer wherever they are. One approval
      authorises one call, a request lapses if nobody answers, a retry reuses
      the waiting request rather than stacking, and an explicit refusal is
      carried through rather than looking like silence
- [x] `keymaker approve` for people without the GUI open

## Phase 5 — Storage backends

- [x] In-memory (tests)
- [x] macOS Keychain
- [x] Linux Secret Service (via `keyring`)
- [x] Windows Credential Manager (via `keyring`)
- [x] **Decided: no restrictive ACL.** The jail closes the same hole from the
      other side, in days rather than weeks, with no Apple Developer ID and
      identically on Linux. A per-item ACL would buy per-secret granularity,
      which nothing in the design asks for: authority is expressed per
      *capability* in an endpoint definition, not per stored value. Revisit only
      if keymaker ever has to defend against a process that is outside the jail
      but still the same user

## Phase 6 — GUI

The library is the contract: the CLI already goes through it and holds no logic
of its own, so the GUI is a second caller rather than a second implementation.
The logic lives in `keymaker_core::gui` and is tested there; the Tauri crate is
ten command wrappers and nothing else.

- [x] Tauri shell, lifting seal's design language (Space Grotesk / DM Sans /
      DM Mono, sharp corners, hairline borders, orange-light / blue-dark)
- [x] Reveal and copy — the **only** place a value can be read, one row at a
      time, and every reveal written to the audit chain
- [x] Task list and run, with output redacted exactly as an agent would get it:
      a person watching a task run has no more need to see the credential
- [x] Endpoint list, marking which ones policy stops for a human
- [x] Audit viewer, showing whether the chain verifies
- [x] Header badge saying whether the jail is actually enforced here
- [x] A regression test that the main Tauri config carries no `devUrl` — see
      below
- [x] An Approvals tab, polled every two seconds with a count in the tab bar —
      an agent blocked on a decision is waiting on a person, so they should not
      have to hit refresh

### The `devUrl` footgun, found by running it

Tauri uses `devUrl` for **any** debug build, not only `tauri dev`. With one in
the main config, `cargo build && ./keymaker-gui` opens a window pointed at a
fixed localhost port and renders whatever is listening there.

That window can call `reveal`, the one command that returns a secret value — so
whoever holds that port controls the UI that reads secrets. It is not
hypothetical: another project on this machine had vite on 5173, 5174 *and* 5175,
and the Keymaker window rendered that project's app.

The main config now has no `devUrl`, so every build loads the bundled frontend.
Dev-server settings are opt-in through `tauri.dev.conf.json`, pinned to
`127.0.0.1` rather than `localhost`, with `strictPort` so a collision fails
loudly. Two tests enforce all of that.

## Deliberately not planned

- `get`, `export`, `reveal`, or any other verb that returns a value to the CLI
- A `read` tool on the MCP surface
- Writing a `.env` file

Each of these is the one change that would give up the claim. `.env` *import*
is fine, and flows the safe direction.
