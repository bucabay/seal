# The agent model — ssh-agent for secrets

Seal's primary user is an AI coding agent, not a human at a prompt. This
document records the decision that follows from taking that seriously, and
states its limits narrowly enough to be defensible.

## The decision

**Seal's CLI has no read verb.** There is no way to ask it for a secret value.

- `seal get` — **removed.**
- `seal export` — **will not be built.**
- The only way to use a secret is to have Seal run something with it.

The model is `ssh-agent`. You do not ask `ssh-agent` for your private key; you
ask it to sign. Nobody considers it crippled for lacking an export verb — the
absence *is* the security property. Seal takes the same trade for API keys,
tokens and passwords.

## Why this model

The controlling question for an agent-facing secrets tool is not where values
rest but whether a value ever enters the model's context window. Anything in
context reaches the transcript, the logs, and possibly a training corpus
(OWASP LLM02), and is exfiltratable by prompt injection (OWASP LLM01).

Today that is governed by a *rule* in `skills/seal/SKILL.md`: "never print a
secret value." A rule is a request. A model that is confused, injected, or
merely sloppy ignores it, and the failure is silent and permanent — once a key
is in a transcript it cannot be recalled.

Deleting the verb ends the negotiation. A capability that does not exist cannot
be misused, cannot be forgotten under context pressure, and cannot be talked
out of a model by a crafted comment in a source file.

### Where the ssh-agent analogy holds, and where it breaks

It holds on the interface: a reference goes in, an *effect* comes out, and the
secret material never crosses back.

It breaks on the mechanism, and the difference must be stated rather than
glossed. `ssh-agent` can refuse to emit the key because it performs the
operation itself — signing happens inside the agent and the key genuinely never
leaves. Seal injects a value into a child process's environment, so **the child
holds the plaintext**. If an agent chooses the child, the agent can reach the
value.

Seal is therefore not an oracle. It is *scoped delivery* with an audited,
human-approved execution boundary. The honest claim is:

> The value never enters the model's context, and every command that touches one
> is recorded and was approved by a human.

Not "the agent could not obtain it by other means." The true oracle — Seal
holding the key and proxying the provider's API — is a different and much larger
product. It is the endgame, not this decision.

## What replaces `get`

`seal run` is the only consuming verb.

```sh
seal run deploy                 # named task from .seal
seal run deploy -e production   # same task, production refs
seal run -- ./scripts/deploy.sh # ad-hoc: approved once, then named (below)
```

Refs resolve through the committed manifest, which holds names and never values:

```toml
# .seal — safe to commit. No secrets, no ciphertext, no key material.
[env.production]
STRIPE_SECRET_KEY = { ref = "stripe/sk_live", approve = true }
DATABASE_URL      = "hardroad/db_url"

[tasks]
deploy  = "./scripts/deploy.sh"
migrate = "prisma migrate deploy"
```

### Ad-hoc commands: trust on first use

Rejecting arbitrary commands outright is too tight a constraint, and constraints
that are too tight get routed around — an agent that cannot run what it needs
goes back to a plaintext `.env`, which is strictly worse than the verb we
removed.

So arbitrary commands are allowed **once**, through an approval that promotes
them into the manifest:

```
$ seal run -- ./scripts/deploy.sh
seal: new command, not in .seal. Approve? This adds:

    [tasks]
    deploy = "./scripts/deploy.sh"

  [y/N]
```

After approval the command is a named task and runs without friction. This is
the SSH known-hosts model, and it buys three things at once: the agent cannot
silently substitute `printenv` for a task a human approved, the manifest gets
authored by the agent and merely reviewed by the human (a one-line diff), and
every command that ever touched a secret is on the record.

`approve = true` on a ref escalates further: production values require a human
tap per use, the equivalent of `ssh-add -c`.

## Output: redaction, not suppression

A rejected alternative, recorded because it is the obvious one: run the child
detached and never return its stdout to the agent.

It does not contain anything. The example that motivates it —
`seal run -- sh -c 'echo $X > /tmp/x'` — defeats it directly, because the agent
reads `/tmp/x` on its next turn. Stdout was never the channel; the filesystem
is, and below that:

| Channel | Cost to a hostile child |
|---|---|
| Write a file the agent reads next | trivial |
| Network egress (`curl`, DNS) | trivial |
| Exit code | ~1 byte per 256 runs — slow, scriptable |
| Filenames, process names, clipboard, timing | open |

Suppression closes one door in a building with no walls. It would only become
containment alongside a sandbox with no network and a filesystem the agent
cannot see — and the tasks that legitimately need secrets (deploy, migrate, call
an API) need network by definition.

It also breaks the workflow Seal exists to serve: an agent that cannot see
whether the test suite passed or why the deploy failed cannot do the job.

So the broker returns stdout and stderr, **redacted**: it knows every value it
just injected and strips those values and their common encodings (base64, URL,
JSON string escaping) before the bytes reach the agent. This is meaningfully
stronger than a terminal-level filter, and still best-effort — a child can
transform a value arbitrarily before printing it. Stated narrowly: *Seal never
returns a value to the model, and filters known values out of what it does
return.*

## How far can redaction go?

Redaction of the returned stream is cheap and worth doing. The question is
whether it can be pushed further — to files the child writes, or to anything
else it leaves behind. The answer is a ladder with a hard ceiling.

**What Seal can redact.** Seal is the parent process for the duration of a run,
so it owns the child's pipes and can filter stdout and stderr before a byte
reaches the agent. That is real and it is the baseline.

**What Seal can scan.** Files are state, not stream — but during a run Seal can
watch for them (FSEvents on macOS, inotify on Linux), and because the agent is
blocked waiting for `seal run` to return, there is a window in which Seal is the
only party that has seen them. Scoped to the working tree and the temp
directory, Seal can scan every file the run touched, scrub any injected value it
finds, fail the run loudly, and tell the human. That catches the realistic
case — an agent that writes a `.env` or a debug log *to be helpful* — before it
is ever read or committed. It is the pre-commit leak scanner, moved earlier.

**What Seal cannot touch.** Once `seal run` returns, Seal is out of the path.
The agent's own file reads, its next shell command, its network access: Seal has
no hook in any of them. Anything that survives the run survives it entirely.

**The ceiling.** Redaction and scanning are pattern matching against an
adversary who picks the encoding. One line defeats them:

```sh
echo "$X" | base64          # not the value
echo "$X" | rev             # not the value
echo "${X:0:16}"; echo "${X:16}"   # neither half is the value
```

This is the same asymmetry that makes data-loss-prevention products ineffective
against a motivated insider, and no amount of engineering closes it. **You
cannot redact your way to containment.**

So redaction and post-run scanning are justified by the accident case, which is
the common one, and must never be described as stopping a determined or injected
agent. What actually closes the gap is making the value useless once taken:

| Control | Stops accidents | Stops deliberate extraction | Cost |
|---|---|---|---|
| Redact returned stdout/stderr | yes | no | low |
| Scan + scrub files touched during the run | yes | no (racy against a hostile child) | moderate |
| Sandbox the child (no network, private fs) | yes | mostly — but breaks deploy, migrate, any API call | high |
| **Ephemeral credentials** (short-lived token minted per run) | yes | **yes — the stolen value expires** | moderate, needs provider support |
| Proxy the provider API (true oracle) | yes | yes — plaintext never leaves Seal | very high, per-provider |

Ephemeral credentials are the honest answer wherever the provider offers them
(AWS STS, GitHub App tokens, Stripe restricted keys, OIDC federation): a value
exfiltrated from a run is worthless minutes later, so the exfiltration channels
above stop mattering without having to close a single one. That is where the
effort belongs once redaction and scanning are in place.

## The human path

Humans still need to read values — to paste a key into a provider's dashboard,
or to check whether the live or test key got stored. That path is the **GUI**,
which already has reveal and copy.

- **GUI = human surface.** Reveal, copy, edit. Requires a window and a click.
- **CLI = agent surface.** No read verb exists.

> The CLI cannot print a secret. The app can — to you.

Two residuals, stated rather than hidden: an agent holding macOS Accessibility
or AppleScript permissions could drive the GUI (a much higher bar, not zero),
and GUI "copy" places the value on a clipboard any process can read.

This makes the GUI load-bearing rather than a nice-to-have. It must be signed
and installable without `xattr -dr`, which is the same Developer ID the
prerequisite below requires — the two costs collapse into one.

## Prerequisite: the keychain ACL

**Everything above is false by inspection until this is fixed.** Items are
created with `security add-generic-password -A` — "allow any application without
warning" (see [DECISIONS.md](DECISIONS.md)). Verified again 2026-09-14 on a
development machine: a value written with `seal set` was read back by a bare
`security find-generic-password` call from an unrelated process, with no prompt
and no denial.

While that holds, removing `get` is an **accident** control, not an **attack**
control:

| Threat | Effect of removing `get` |
|---|---|
| Agent runs `seal get X` "to check it", value lands in transcript forever | **Eliminated.** The dominant real-world failure mode. |
| Deliberate extraction — prompt injection, or a model routing around a constraint | **None.** `security find-generic-password` walks around Seal entirely. |

Accidents vastly outnumber attacks, so the removal is worth doing on its own
merits and on that basis alone. But the public claim stays narrow until Seal
ships one signed binary with a designated requirement and creates items with an
ACL bound to that identity.

Linux is weaker still and we say so publicly rather than imply parity: the
Secret Service unlocks per session and any same-user process can talk to it. The
honest Linux claim is "the value never enters the model's context," never "the
agent could not obtain it another way."

## Consequences

- `seal get` is gone; every script and skill that used
  `export TOKEN="$(seal get …)"` must move to `seal run`.
- There is no bulk read, so there is no `seal export` and no "write a `.env`
  file" — those are `get` in a larger wrapper. `.env` **import** remains, since
  it flows the safe direction.
- `seal run` must ship **before** `get` is removed; deleting the only consuming
  verb with no replacement leaves the tool unusable.
- The MCP surface is `list` / `run` / `set` / `delete` / `request_approval`.
  There is no `read`. A tool that returns a value is the one thing that cannot
  be added later without giving up the claim.

## Open questions

- Does the approval prompt fire often enough to be ignored (click-through
  fatigue), and does a per-repo `.seal` diff stay small enough to actually read?
- Is redaction worth the complexity given it is defeated by any encoding
  transform, or is the honest move to redact nothing and say so?
- Does "agents only" remove the adoption path, given humans usually adopt a tool
  first and point agents at it afterwards? The GUI is the answer, which makes
  signing it urgent.
- Should effort skip redaction entirely and go straight to ephemeral credentials,
  which are the only control on the ladder that survives a hostile child? The
  counter is that they need per-provider work and cover only providers that mint
  short-lived tokens, leaving static API keys — the common case — unaddressed.
