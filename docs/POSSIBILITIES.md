# Possibilities

A from-scratch exploration of market, ideal customer, and candidate
architectures. **None of this is decided** — these are possibilities to weigh
against the current design (see [ARCHITECTURE.md](ARCHITECTURE.md) and
[RESEARCH.md](RESEARCH.md)). Facts verified 2026-08.

## Market: two incumbent camps and a gap

Two adjacent categories of tooling exist, with an unowned space between them.

**Camp A — centralized secret managers** (Doppler, Infisical, Vault, native
cloud stores). They solve *storage and team sync*. Their agent integrations
return secret **values to the model context** (Doppler's MCP exposes "Read
secret values"). Moat: scale + SOC 2. Their architecture does not target the
"agent shouldn't see it" problem.

**Camp B — agent-security guard tools** (e.g. `medusa`, `hol-guard`,
`ship-safe`, `prismor`, `toolport`, `mcp-audit`). Fast-moving scanners,
gateways, HITL approval planes, and MCP-config auditors. They **observe and
block** risky behavior; they do not hold or deliver secrets.

**The gap:** no one owns *store the secret AND deliver it to an agent such that
the agent never possesses it*. Camp A delivers values; Camp B watches but cannot
deliver. This maps onto OWASP LLM01 (prompt injection) and LLM02 (sensitive
information disclosure).

## Ideal customer (one possibility)

> **The AI-coding lead.** An individual senior engineer or a 2–20-person team
> that runs coding agents (Claude Code, Cursor, Copilot, opencode) daily,
> deploys to Cloudflare/Vercel/AWS, and works in devcontainers + CI. They have
> been burned by — or worry about — a key appearing in a transcript, a hook, or
> a committed `.env`. They want local-first, no third party, and "works in my
> editor and CI." They pay when the *team* version (shared vaults, audit,
> approval) appears.

Jobs-to-be-done:

1. Store a secret once; let the agent use it in every tool without pasting it.
2. Guarantee the value never appears in a transcript, log, or context.
3. Work identically on a laptop, devcontainer, and CI.
4. Know which agent used which secret, and block/approve the dangerous ones.

## Candidate architectures

### Possibility A — SaaS-centralized (Doppler-style)

Secrets in the vendor's cloud; agents pull via MCP/API. Right for
enterprise/audit; poor fit for a small builder (SOC 2, sales, and it is the
incumbent's game). Values-to-context is the wrong model for the agent problem.

### Possibility B — pure local CLI + keychain (today's Seal)

Simplest, but structurally leaks: `seal get` prints to stdout, so the value
reaches the transcript. No identity, scoping, approval, or audit. Dead-ends at
solo-local.

### Possibility C — local-first secrets broker

Separate **possession from use**: the agent is untrusted (prompt-injectable);
a trusted local **broker** is the only process that holds plaintext. The agent
asks the broker to *use* a secret; the broker injects it at the execution
boundary and redacts the output.

```
   LLM (untrusted)                     Broker (trusted)                 Storage
 ┌──────────────┐   MCP/CLI        ┌────────────────────┐        ┌──────────────────┐
 │  Claude Code │ ── run --with ─► │  seal daemon       │  pull  │ local encrypted   │
 │  Cursor      │    ref, not value│  • decrypts/holds  │ ─────► │  store (default)  │
 │  Copilot     │ ◄── redacted ────│  • policy/audit    │        │  OS keychain/TPM  │
 └──────────────┘    output        │  • injects env     │        │  cloud stores     │
                                   └────────────────────┘        │  relay (later)    │
                                        spawns child ───────────►└──────────────────┘
                                        process w/ secret in env
```

Components:

1. **Broker daemon** — single static binary (Rust/Go), sidecar next to the agent
   wherever it runs. Sole plaintext holder; decrypts on demand into memory.
2. **Pluggable storage trait** (`get/set/list`) — default is a local encrypted
   store with the master key in the OS keychain/TPM; GCP/AWS/Cloudflare and a
   relay are later backends.
3. **MCP surface = capabilities, not values:** `list` (names/refs only),
   `run --with <ref> -- <cmd>` (injects into child env; returns stdout
   *redacted*), `set`/`delete`, `approve`. **No `read`/`get` tool.**
4. **Policy + audit** — scoping (project/vault), identity (which agent),
   sensitivity tiers (prod keys → human approval), and a log of *which ref* was
   used, never the value.
5. **CLI** for humans and bootstrap.

Under this model the earlier "three contexts" (local / remote-we-control /
ephemeral agent container) collapse into one: the broker is always a sidecar;
only the unlock differs — OS keychain/TPM on desktop, age identity or OIDC in a
container.

The one-line claim if this is pursued: *"Your agent runs with secrets, never
sees them."*

## Trade-offs at a glance

| Dimension | A (SaaS) | B (CLI+keychain) | C (broker) |
|---|---|---|---|
| Team sync / audit | strong | none | additive later |
| Agent-safe by construction | no (values to context) | no (stdout leak) | yes |
| Effort / capital | high (SOC2, sales) | lowest | medium |
| Portability to containers | via cloud | poor | sidecar + unlock |
| Differentiated claim | none (incumbent) | weak | strong |

## Open questions (not decided)

- Does the customer actually demand "never in context," or is "stop me from
  leaking secrets" (Camp B's territory) the real job?
- Is the guard-tool camp (Camp B) a competitor, an integration target, or both?
- Does the revenue moment (team vaults / audit / approval) justify the broker's
  added complexity over today's CLI?
- Which storage backend ships first if the broker is pursued: encrypted file,
  or keep the OS keychain as the default store?
