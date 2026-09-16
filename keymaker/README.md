# keymaker

Secrets your agent can **use** but never **read**.

```sh
keymaker get stripe/sk_live
# keymaker: there is no `get`. That is the point.
```

The Keymaker does not hand over a master key. He cuts one key, for one door,
used once. That is what this is: an agent asks for an *action*, not a value.

## Why

An AI agent that can read a secret will eventually print one — into a
transcript, a log, a bug report, or a training corpus. Today most tools address
that with a *rule* ("never print a secret value"), which a confused, injected,
or sloppy model ignores, silently and irreversibly.

keymaker removes the capability instead. Three mechanisms, applied together:

| | What it does | What it closes |
|---|---|---|
| **1. Jail** | The agent runs inside an OS sandbox that denies the keychain, `~/.ssh`, `~/.aws`, cloud metadata | The agent reading the store directly |
| **2. Broker** | Values are injected into a *task* or attached to a *pinned request*, never into the agent | The agent's process ever holding plaintext |
| **3. Expiry** | Credentials are minted per call, scoped and short-lived where the provider allows | A stolen value staying useful |

Each closes a different door. None is sufficient alone. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Install

```sh
cargo install --path crates/keymaker-cli      # from this repo
```

## Use

```sh
# Start the broker. It is the only process that holds a plaintext value.
keymaker serve &

# Confine an agent. Everything it spawns inherits this.
keymaker jail -- claude

# Store a value. It is read from stdin and never echoed.
printf 'sk_live_…' | keymaker set stripe/sk_live

# Run a task from .keymaker with secrets in *its* environment.
keymaker run deploy -e production

# Run something ad-hoc. Asked once, then it is a named task forever.
keymaker run -- ./scripts/seed.sh

# Send a request built from a pinned endpoint definition.
keymaker call stripe.refund '{"amount": 500, "charge": "ch_1"}'

# Send a request built from a pinned endpoint definition.
keymaker call github.issue_create '{"title": "from an agent"}'

# Answer something an agent is blocked on.
keymaker approve                  # or use the GUI's Approvals tab

# Names only — never values.
keymaker list
keymaker audit --verify           # the trail, and proof it has not been edited
keymaker doctor -e production     # which references are missing here
```

There is no `get` and no `export`. To read a value yourself, use the GUI.

## Endpoint definitions

`endpoints/` ships definitions for Anthropic, OpenAI, Stripe, GitHub and
Cloudflare. A definition is a security control, not glue — it decides what a
credential may be used for:

```toml
[[endpoint]]
name = "stripe.refund"
method = "POST"                       # pinned
host = "api.stripe.com"               # pinned
path = "/v1/refunds"                  # pinned
secret = "stripe/sk_live"             # a name, never a value
body_format = "form"
schema = { charge = "string", amount = "uint32?" }
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
policy = { step_up = "amount > 100000" }   # a big refund stops for a human
```

The same Stripe key is read-only for `stripe.charge_get`, needs approval above a
threshold for `stripe.refund`, and always needs a human for
`stripe.payout_create`. That asymmetry is the whole point.

## The manifest

`.keymaker` is safe to commit. It holds names, never values.

```toml
version = 1

[tasks]
deploy = "./scripts/deploy.sh"

[env.production]
DATABASE_URL      = "hardroad/db_url"
STRIPE_SECRET_KEY = { ref = "stripe/sk_live", approve = true }
```

An ad-hoc command is allowed once, through an approval that writes it here.
After that the agent can run `deploy` but cannot substitute `printenv`, because
it is naming a task rather than choosing a command.

## Library

The CLI is a thin wrapper. Everything is in `keymaker-core`, so the GUI uses the
same code:

```rust
use keymaker_core::{manifest::Manifest, runner::{Runner, ProcessSpawner}};
use keymaker_core::store::KeychainStore;

let store = KeychainStore::default();
let runner = Runner::new(&store, &ProcessSpawner);
let outcome = runner.run_task(&manifest, "deploy", "production")?;
println!("{}", outcome.stdout_string());   // already redacted
```

## Short-lived credentials

Where a provider will mint one, keymaker exchanges the long-lived secret for a
credential that expires — and then a stolen value is worthless before it is
useful, without having to close a single exfiltration channel:

| Provider | Stored secret | What the task gets |
|---|---|---|
| AWS | access key | an STS role session, signed for with SigV4 |
| GitHub | App private key | an installation token, an hour, narrowable to one repo |
| Any RFC 8693 STS | subject token | a scope-, audience- and time-narrowed token |

In each case the stored secret signs or is exchanged; it never travels.

A static API key has nothing to shorten, and for those the broker (mechanism 2)
is the only answer. `Acquired` says which of the two happened in the type, so a
static fallback is never silently glossed.

## What is honest about this

- The jail is an enforcement layer, not a proof. It does not survive local root,
  and on macOS a denied operation is usually dropped silently rather than
  erroring, so a confined tool misbehaves rather than failing loudly.
- Redaction catches accidents, not attackers. `base64`, `rev`, or printing a
  value in two halves all defeat it, and nothing fixes that.
- Only short-lived credentials survive a hostile task, and they need the
  provider to mint them. A static API key has nothing to shorten.

Limits are stated in full in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Status

Early. See [docs/PLAN.md](docs/PLAN.md) for exactly what is built and what is
not. 331 tests, `cargo test --workspace`.

## License

MIT
