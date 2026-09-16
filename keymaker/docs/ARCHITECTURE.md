# Architecture

keymaker is one library (`keymaker-core`) with thin callers. The CLI is a
caller. The GUI will be a caller. Nothing important lives in either.

```
crates/
  keymaker-core/     every decision, every test
  keymaker-cli/      argument parsing and printing, nothing else
```

## The three mechanisms

An agent gets a secret out in exactly three ways. Each is closed by a different
mechanism, and none of the three is sufficient alone.

| | Agent reads the store | Task holds plaintext | Stolen value stays useful |
|---|---|---|---|
| 1. Jail (`jail`) | **closed** | open | open |
| 2. Broker (`handle`, `runner`, `provider`) | open | **closed** | open |
| 3. Expiry (`creds`) | open | open | **closed** |

### 1 — Jail (`jail.rs`)

Confine the agent rather than hiding the secret. The sandbox denies reads of
the OS secret store, `~/.ssh`, `~/.aws`, `~/.config/gcloud`, `~/.kube`,
`~/.netrc`, `~/.gnupg`, and denies *executing* `security`, `secret-tool` and
`gpg` — denying the paths is not enough while the tools that read them can run.
Children inherit the sandbox, so the agent cannot shell out to escape.

Two modes:

- **Shield** — allow by default, deny the credential paths and tools. An agent
  keeps working and the known-sensitive things are closed. This is the default.
- **Strict** — deny by default, allow only what is listed. Much stronger, much
  more likely to break ordinary work.

macOS renders a seatbelt profile applied through `sandbox-exec`. Linux gets a
`LandlockPlan` describing what Landlock (filesystem), seccomp (syscalls) and a
network namespace (metadata endpoints) must each enforce.

Two things learned by building it, both now covered by tests:

- **A profile that does not load protects nothing.** An early version contained
  every expected substring and was still rejected outright, because sbpl's
  `remote ip` filter accepts only `*` or `localhost` — a specific address such
  as `169.254.169.254` makes the whole profile fail. Metadata blocking is
  therefore *not* available on macOS; `Profile::blocks_hosts()` says so rather
  than letting a caller assume otherwise.
- **Deny-by-default kills the loader.** dyld maps the shared cache and stats
  `/` on the way to every library. Without `file-map-executable`,
  `file-read-metadata` and read access to `/`, a strict profile loads cleanly
  and then aborts every process it wraps, which reads as a crash rather than a
  policy decision.

### 2 — Broker

Three pieces, none of which returns a value to the caller.

**`handle.rs` — capability handles.** The agent never gets a durable reference
like `stripe/sk_live`; a durable reference leaks usefully. It gets a handle
bound to a session, a tool-call epoch, a sequence number, a TTL and a use
quota. Redeeming it causes the broker to act. A spent handle and a handle that
never existed return the same error, so the agent cannot probe for handles
belonging to another session. Ordering enforcement is opt-out, because a
multi-use handle must be able to retry.

**`runner.rs` — named tasks.** The agent asks for a task by name. The runner
resolves the manifest's bindings, reads the values, spawns the command with
them in *its* environment, and returns output with those values filtered out.
An unapproved command is refused before anything is read or spawned.

**`provider.rs` — constrained request constructors.** Not a forwarding proxy. A
proxy that attaches a credential and forwards whatever it is given lets an
injected agent point your key at any endpoint on that host. A constructor pins
method, host and path, allowlists headers, bounds the body, validates it
against a schema, and only then attaches the credential.

The separation is enforced by the types. `check()` returns a `CheckedRequest`
that structurally cannot contain a secret; `prepare()` takes the value as an
argument, so nothing in the module can reach the store on its own.

### 3 — Expiry (`creds.rs`)

Rather than keeping an injected value in, mint one that expires. `Source::acquire`
returns `Acquired::Minted` or `Acquired::Static` — the distinction is in the
type, because only one of them expires, and a static fallback carries a warning
rather than being glossed over. `require_ephemeral` refuses the fallback
entirely.

## Supporting pieces

- **`store.rs`** — `Secret` has no `Display` and a `Debug` that prints
  `Secret([redacted; N bytes])`, so a value cannot reach a log or an error by
  accident. The only way out is `expose()`, which is greppable. Backends:
  in-memory, macOS Keychain.
- **`manifest.rs`** — the committed `.keymaker`. Names only. Holds tasks,
  environments with inheritance, and the trust-on-first-use approval flow that
  promotes an ad-hoc command into a named task.
- **`redact.rs`** — filters injected values and their encodings (base64 in four
  variants, percent-encoding, JSON escaping) out of a stream, including across
  chunk boundaries.
- **`policy.rs`** — allow, deny, or step-up. Deny beats step-up; an unparseable
  rule denies rather than being skipped.
- **`audit.rs`** — hash-chained entries. Editing or removing one breaks the
  chain and `verify` says where. A test asserts no event variant can carry a
  value.
- **`clock.rs` / `id.rs`** — time and entropy are injected, so expiry and
  handle generation are testable without sleeping or flaking.

## What this does not do

Stated plainly, because the value of the project is that the claims are narrow
enough to be true.

- **The jail is enforcement, not proof.** It does not survive local root outside
  the sandbox, and profile authoring is a trade: too tight and ordinary work
  breaks, too loose and the point is lost.
- **Redaction catches accidents, not attackers.** `echo "$X" | base64`, `rev`,
  or printing the value in two halves all defeat it. A test asserts this limit
  so that nobody later claims more.
- **A named task still holds plaintext.** The agent's process does not, which is
  the point — but the task does, so only expiry (3) helps against a task that is
  itself hostile.
- **Only the HTTP path keeps a static key out of the child entirely.** Anything
  that falls back to a named task is relying on mechanisms 1 and 3.
