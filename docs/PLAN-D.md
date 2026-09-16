# Plan D — three ways to use a secret without seeing it

The problem in one line: an agent must be able to *use* a credential while
having no way to *obtain* it.

There are only three mechanisms that actually achieve this. Everything else —
redaction, output scanning, naming conventions, rules in a skill file — is an
accident control. It helps with the common case, where a model prints something
it shouldn't, and does nothing against a model that has been injected or is
simply working around a constraint.

The three approaches are independent. They can be built in any order and they
stack.

---

## Approach 1 — Confine the agent

**Idea:** don't hide the secret from the agent. Take away the agent's ability to
reach the place secrets live.

The agent process runs inside a kernel-enforced sandbox that denies reads of:

- the OS secret store (`~/Library/Keychains`, the Secret Service socket)
- the tools that talk to it (`security`, `secret-tool`)
- other credential locations (`~/.ssh`, `~/.aws`, `~/.config/gcloud`, `~/.kube`)
- cloud instance metadata (`169.254.169.254`)

Everything the agent spawns inherits the sandbox, so it cannot shell out to
escape it.

### How it works

**macOS** — a seatbelt profile applied with `sandbox_init()` at process start.
Deny-by-default, with explicit allows for the paths the agent legitimately needs.
Once applied, a process cannot escape its own profile, even if it gains root
*inside* the sandbox. Note that seatbelt usually drops a denied operation
silently rather than returning an error, so tools behave oddly rather than fail
loudly.

**Linux** — three separate mechanisms together:
- **Landlock** for filesystem rules (`LANDLOCK_ACCESS_FS_READ_FILE` on the paths
  above)
- **seccomp-BPF** to filter syscalls
- a **network namespace** to block metadata endpoints

### What it stops

The bypass that currently defeats everything else. Today any process running as
the user can do:

```sh
security find-generic-password -s seal -a "stripe:sk" -w    # no prompt
```

Inside the sandbox, that call fails. This is the single largest gap in the
current design, and closing it costs days rather than weeks.

### What it does not stop

The task still receives the plaintext. If the agent can read the task's output
or the files it writes, it can still recover the value. This approach secures
the *store*, not the *use* — it needs Approach 2 to close that half.

It is also an enforcement layer, not a proof. It does not survive local root
outside the sandbox, and profile authoring is fiddly: too tight and normal work
breaks, too loose and the point is lost.

### Cost

Low. A profile per platform, applied at agent launch. No protocol design, no new
daemon, no change to how secrets are stored.

---

## Approach 2 — A broker that acts on the agent's behalf

**Idea:** one trusted process holds every plaintext value. The agent never
receives a value, only permission to have an action performed.

### How it works

A user-session daemon listens on a unix socket. The agent's client binary holds
no key material — it carries a request.

**Identifying the caller.** The kernel supplies the peer's UID, GID and PID
(`SO_PEERCRED` on Linux, `LOCAL_PEERCRED` / `getpeereid` on macOS/BSD). The
caller cannot forge these; they come from kernel bookkeeping, not from anything
sent over the wire. Use a unix socket, not stdio, for exactly this reason.

One real bug to design around: **PID reuse**. A caller can connect, exit, and
have its PID reassigned to a different process before the broker finishes
checking. On Linux, hold a `pidfd` — it names a specific process for its
lifetime, not a number that gets recycled. Elsewhere, re-check liveness after
the credential check and fail if anything changed.

**Handing out permission, not references.** A reference like `stripe/sk_live` is
durable: leak it and it keeps working. Instead issue a one-use handle bound to:

- the session and the current tool-call
- a fresh nonce and a monotonically increasing sequence number
- a TTL in seconds
- a call quota

A stolen handle is already dead. There is nothing durable to exfiltrate.

**Two shapes of execution.**

*Named commands.* For work that isn't an API call — `./deploy`,
`prisma migrate`. The command comes from a committed manifest, so the agent
cannot substitute `printenv` for a task a human approved. A new command is
approved once and written into the manifest; after that it runs freely. Output
comes back redacted.

*Constrained HTTP requests.* Not a forwarding proxy — a request **constructor**.
The definition pins the method, host and path, allowlists headers, bounds the
body size, and validates the body against a schema. Only if every check passes
does the broker attach the credential and send the request:

```toml
[provider.stripe.refund]
method = "POST"
host   = "api.stripe.com"
path   = "/v1/refunds"
inject = { header = "Authorization", format = "Bearer {ref}" }
ref    = "stripe/sk_live"
schema = { amount = "uint32", charge = "string" }
policy = { step_up = "amount > 100000" }
```

The distinction matters. A forwarding proxy lets an injected agent point your
key at any endpoint on that host. A constructor fails closed on a constraint
violation — *before* the credential is touched.

### What it stops

This is the only approach where a **static, non-expiring** credential never
enters the agent's process tree at all. `printenv` finds nothing, because
nothing was injected. Encoding tricks (`base64`, `rev`, splitting the string)
are irrelevant, because the process never held the value to transform.

It also gives two things away for free, since the broker is already the only
component that sees plaintext:

- **policy** — allow, deny, or require a human tap, per call
- **audit** — a hash-chained log of what was requested and decided, holding
  request structure and outcome but never values

Measured overhead for mediated HTTP is around **0.15 ms** per call. Latency is
not a reason to avoid this.

### What it does not stop

Anything outside a definition falls back to named commands, where the task does
hold the plaintext. The broker is a new trusted component: a bug in it is a
total compromise. And per-provider definitions are ongoing work.

That ongoing work is worth reframing, though. Because the definition *is* the
enforcement policy — not integration glue — each one added is a security control
gained, and a project that needs its own API covered has a direct reason to write
and share one.

### Cost

High. A daemon, a socket protocol, a handle format, a policy engine, and one
definition per provider.

---

## Approach 3 — Make the stolen value worthless

**Idea:** stop trying to keep the value in. Make it expire before it is useful.

If the credential injected into a task is valid for five minutes and scoped to
one operation, then every exfiltration channel — files, network, exit codes,
timing — stops mattering. You do not have to close a single one.

### How it works

The broker holds one long-lived root credential and never hands it out. At the
moment of use, it exchanges that root for a narrow one, narrow in three
dimensions at once:

- **scope** — only the permissions this one call needs
- **audience** — bound to exactly the service being called
- **time** — measured against the operation, so minutes rather than a session

The standard mechanism is **OAuth 2.0 Token Exchange (RFC 8693)**, which also
carries `actor_token` and `may_act` claims, so the resulting token records that
one party acted on behalf of another — that is what makes "who authorised this"
answerable afterwards.

Where there is no STS, providers usually offer a native equivalent: AWS STS
`AssumeRole`, GitHub App installation tokens, Stripe restricted keys, GCP
short-lived service account credentials.

**Hardware, at rest.** A Secure Enclave or TPM cannot store an arbitrary API
key — it holds only its own generated keypair. But that keypair can *wrap*
(encrypt) the vault, and the wrapping key never leaves the hardware. Unwrapping
then requires the enclave, which can be made to require user presence. That
turns "approve this use" into a biometric touch rather than a prompt a process
can click through.

### What it stops

Theft of the injected value, completely — after the TTL, a stolen credential is
inert. This is the only approach that keeps working when the agent has full
filesystem and network access, and it is what makes CI and ephemeral containers
tractable, since they can prove an identity rather than carry a secret.

### What it does not stop

It needs the provider to support it. A Stripe live key, a Postgres password, a
plain API key with no permission model — none of these have anything to shorten.
For those, only Approach 2 helps.

Within the TTL the credential is fully usable, so this reduces the window rather
than closing it.

### Cost

Medium, and per-provider. The exchange logic is small; knowing each provider's
scope model is the work.

---

## How they combine

Each one closes a different door. None is sufficient alone.

| | Agent reads the store directly | Task holds plaintext | Stolen value stays useful |
|---|---|---|---|
| 1. Confine the agent | **closed** | open | open |
| 2. Broker | open | **closed** (HTTP path) | open |
| 3. Short-lived credentials | open | open | **closed** |
| All three | closed | closed | closed |

Read across the top row: those are the three ways a secret actually gets out.
Read down a column: only one approach closes each.

## Implementation

Built as a separate project rather than bolted onto seal, because the agent-only
constraint (no read verb at all) is incompatible with seal's human-facing CLI.
See [`keymaker/`](../keymaker/) — `keymaker-core` is the library, `keymaker-cli`
the binary, and a GUI will be a third caller of the same library.

## What to build first

1. **Approach 1.** Days of work, closes the largest hole, and makes every other
   claim in these docs true rather than conditional. Nothing above it is real
   until the raw `security` call fails.
2. **Approach 2, named commands only.** This is the replacement for `seal get`
   (see [AGENT-MODEL.md](AGENT-MODEL.md)), so it has to exist before that verb
   can be removed.
3. **Approach 2, constrained HTTP** — starting with whichever APIs get called
   most. This is the only answer for static keys.
4. **Approach 3**, for the providers that support it.

## Open questions

- How tight can the sandbox profile be before ordinary work breaks? This is
  empirical and needs a week of real use to answer.
- Do one-use handles survive real agent loops, where a tool call may legitimately
  retry?
- Is a schema per endpoint too much to ask of whoever writes a definition, or is
  it exactly what makes the definition worth sharing?
