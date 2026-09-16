//! keymaker — the CLI.
//!
//! There is no `get`. There is no `export`. A secret can be stored and it can
//! be used; it cannot be read back. See docs/AGENT-MODEL.md in the seal repo
//! for why that is the whole point.
//!
//! Every command here is a thin wrapper over `keymaker-core`, so the GUI can
//! do exactly the same things through the same library.

use keymaker_core::clock::SystemClock;
use keymaker_core::jail::Profile;
use keymaker_core::manifest::Manifest;
use keymaker_core::provider::{Catalog, RequestDraft};
use keymaker_core::runner::{ProcessSpawner, Runner};
use keymaker_core::store::{Secret, SecretStore};
use keymaker_core::Error;
use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

const USAGE: &str = "\
keymaker — secrets your agent can use but never read

USAGE
  keymaker serve                   run the broker; the only holder of plaintext
  keymaker mcp                     speak MCP on stdio, for an agent to connect to
  keymaker jail -- <command>        run a command confined; it cannot reach the store
  keymaker jail --print            print the sandbox profile that would be applied
  keymaker run <task> [-e <env>]   run a task named in .keymaker
  keymaker run -- <command>        run an ad-hoc command (asks once, then remembers)
  keymaker call <endpoint> [json]  send a request built from an endpoint definition
  keymaker list                    tasks, endpoints and references — names only
  keymaker doctor [-e <env>]       which references are missing on this machine
  keymaker set <ref>               store a value, read from stdin, never echoed
  keymaker rm <ref>                remove a value
  keymaker audit [--verify]        show the audit log

There is deliberately no `get` and no `export`: the CLI cannot print a secret.
Use the GUI to read a value yourself.

The broker is used when one is running; otherwise commands run in-process and
do the same checks. `keymaker serve` is what an agent talks to.

OPTIONS
  -e, --env <name>    environment from .keymaker (default: \"default\")
  -f, --file <path>   manifest path (default: ./.keymaker)
  -y, --yes           approve a new ad-hoc command without prompting
";

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("keymaker: {}", msg);
    std::process::exit(1);
}

struct Args {
    env: String,
    manifest: PathBuf,
    yes: bool,
    rest: Vec<String>,
}

/// Splits flags from positional arguments, stopping at `--` so a wrapped
/// command keeps its own flags.
fn parse(argv: Vec<String>) -> Args {
    let mut a = Args {
        env: std::env::var("KEYMAKER_ENV").unwrap_or_else(|_| "default".into()),
        manifest: PathBuf::from(".keymaker"),
        yes: false,
        rest: Vec::new(),
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--" => {
                a.rest.push("--".into());
                a.rest.extend(argv[i + 1..].iter().cloned());
                return a;
            }
            "-e" | "--env" => {
                i += 1;
                a.env = argv
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| die("--env needs a value"));
            }
            "-f" | "--file" => {
                i += 1;
                a.manifest = PathBuf::from(
                    argv.get(i)
                        .cloned()
                        .unwrap_or_else(|| die("--file needs a path")),
                );
            }
            "-y" | "--yes" => a.yes = true,
            other => a.rest.push(other.to_string()),
        }
        i += 1;
    }
    a
}

fn load_manifest(path: &Path) -> Manifest {
    match std::fs::read_to_string(path) {
        Ok(src) => Manifest::from_toml(&src).unwrap_or_else(|e| die(e)),
        Err(_) => Manifest::from_toml("version = 1").expect("empty manifest is valid"),
    }
}

fn save_manifest(path: &Path, m: &Manifest) {
    let text = m.to_toml().unwrap_or_else(|e| die(e));
    std::fs::write(path, text)
        .unwrap_or_else(|e| die(format!("writing {}: {}", path.display(), e)));
}

fn load_catalog() -> Catalog {
    for p in [
        PathBuf::from(".keymaker.endpoints.toml"),
        config_dir().join("endpoints.toml"),
    ] {
        if let Ok(src) = std::fs::read_to_string(&p) {
            return Catalog::from_toml(&src).unwrap_or_else(|e| die(e));
        }
    }
    Catalog::default()
}

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/keymaker")
    } else {
        PathBuf::from(home).join(".config/keymaker")
    }
}

/// The real store on macOS; elsewhere an in-memory stand-in until the
/// platform backend lands, so the CLI is still exercisable.
fn open_store() -> Box<dyn SecretStore> {
    #[cfg(target_os = "macos")]
    {
        Box::new(keymaker_core::store::KeychainStore::default())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(keymaker_core::store::MemoryStore::new())
    }
}

fn cmd_jail(args: &Args) {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    let profile = Profile::shield(&cwd).deny_path("~/.keymaker");

    let after_sep = args.rest.iter().position(|a| a == "--");
    match after_sep {
        None => {
            // `--print` or nothing: show what would be enforced.
            println!("{}", profile.to_seatbelt());
            if !cfg!(target_os = "macos") {
                let plan = profile.to_landlock_plan();
                println!(
                    "\n; linux plan\n{}",
                    serde_json::to_string_pretty(&plan).unwrap()
                );
            }
        }
        Some(i) => {
            let command: Vec<String> = args.rest[i + 1..].to_vec();
            if command.is_empty() {
                die("jail needs a command after `--`");
            }
            let Some(argv) = profile.wrap_command(&command) else {
                die("no sandbox mechanism on this platform yet; see docs/PLAN.md");
            };
            let status = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .status()
                .unwrap_or_else(|e| die(format!("launching sandbox: {}", e)));
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

fn cmd_run(args: &Args) {
    let mut manifest = load_manifest(&args.manifest);
    let store = open_store();
    let spawner = ProcessSpawner;
    let runner = Runner::new(store.as_ref(), &spawner).watching_defaults();

    let sep = args.rest.iter().position(|a| a == "--");
    let outcome = match sep {
        // `run -- <command>`: ad-hoc, approved once then remembered.
        Some(i) => {
            let command = args.rest[i + 1..].join(" ");
            if command.trim().is_empty() {
                die("run needs a command after `--`");
            }
            if let Some(proposal) = manifest.propose(&command) {
                if !approve(&proposal.snippet(), args.yes) {
                    die("not approved");
                }
                manifest.approve(&proposal).unwrap_or_else(|e| die(e));
                save_manifest(&args.manifest, &manifest);
                eprintln!("keymaker: approved as task `{}`", proposal.name);
            }
            runner.run_command(&manifest, &command, &args.env)
        }
        None => {
            let Some(task) = args.rest.first() else {
                die("run needs a task name")
            };
            // Prefer a running broker: there the value never enters this
            // process at all. Without one, fall through and do the same work
            // here, which is still safe for a human at a terminal.
            if let Some(mut client) = broker_client() {
                match client.grant_and_run(task, &args.env) {
                    Ok(resp) => report(resp),
                    Err(e) => die(e),
                }
            }
            runner.run_task(&manifest, task, &args.env)
        }
    };

    match outcome {
        Ok(out) => {
            std::io::stdout().write_all(&out.stdout).ok();
            std::io::stderr().write_all(&out.stderr).ok();
            if out.redacted {
                eprintln!("keymaker: a value was printed by this task and has been masked");
            }
            for f in &out.files_with_values {
                eprintln!("keymaker: this task wrote a credential to {}", f.display());
            }
            std::process::exit(out.exit_code.unwrap_or(1));
        }
        Err(e) => die(e),
    }
}

fn approve(snippet: &str, assume_yes: bool) -> bool {
    if assume_yes {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        eprintln!("keymaker: new command needs approval, but stdin is not a terminal.");
        eprintln!("re-run with --yes, or approve it once interactively.");
        return false;
    }
    eprintln!("keymaker: new command. Approving adds this to .keymaker:\n");
    eprintln!("{}", snippet);
    eprint!("approve? [y/N] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes")
}

/// Print a broker response and exit with a code that matches it.
fn report(resp: keymaker_core::protocol::Response) -> ! {
    use keymaker_core::protocol::Response;
    match resp {
        Response::Ran {
            exit_code,
            stdout,
            stderr,
            redacted,
            leaked_files,
        } => {
            print!("{}", stdout);
            eprint!("{}", stderr);
            if redacted {
                eprintln!("keymaker: a value was printed by this task and has been masked");
            }
            for f in &leaked_files {
                eprintln!("keymaker: this task wrote a credential to {}", f);
            }
            std::process::exit(exit_code.unwrap_or(1));
        }
        Response::Called {
            status,
            body,
            redacted,
        } => {
            println!("{}", body);
            if redacted {
                eprintln!("keymaker: the response echoed the credential; it has been masked");
            }
            std::process::exit(if (200..300).contains(&status) { 0 } else { 1 });
        }
        Response::ApprovalRequired { capability, rule } => {
            eprintln!("keymaker: `{}` needs approval ({}).", capability, rule);
            eprintln!("Approve it in the GUI, or call again once approved.");
            std::process::exit(3);
        }
        Response::Names { names } => {
            for n in names {
                println!("{}", n);
            }
            std::process::exit(0);
        }
        Response::Error { kind, message } => die(format!("{}: {}", kind, message)),
        other => die(format!("unexpected reply: {:?}", other)),
    }
}

fn cmd_call(args: &Args) {
    let Some(name) = args.rest.first() else {
        die("call needs an endpoint name")
    };

    let body_src = match args.rest.get(1) {
        Some(s) => s.clone(),
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf).ok();
            if buf.trim().is_empty() {
                "{}".into()
            } else {
                buf
            }
        }
    };
    let body: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&body_src).unwrap_or_else(|e| die(format!("body is not JSON: {}", e)));
    let draft = RequestDraft {
        endpoint: name.clone(),
        body,
        ..Default::default()
    };

    // With a broker, the credential never enters this process.
    if let Some(mut client) = broker_client() {
        match client.grant_and_call(name, draft) {
            Ok(resp) => report(resp),
            Err(e) => die(e),
        }
    }

    // Without one, run the same sequence here: check, decide, then inject.
    let catalog = load_catalog();
    let endpoint = catalog.get(name).unwrap_or_else(|e| die(e));
    let checked = endpoint.check(&draft).unwrap_or_else(|e| die(e));
    match endpoint.decide(&checked) {
        keymaker_core::policy::Decision::Allow => {}
        keymaker_core::policy::Decision::Deny(why) => die(format!("denied by policy: {}", why)),
        keymaker_core::policy::Decision::StepUp(why) => {
            let prompt = format!("policy requires approval: {}\n{}\n", why, checked.url());
            if !approve(&prompt, args.yes) {
                die("not approved");
            }
        }
    }

    let store = open_store();
    let secret = store.get(&endpoint.secret).unwrap_or_else(|e| die(e));
    let prepared = endpoint.prepare(checked, secret.expose());

    let transport = keymaker_core::transport::HttpTransport::default();
    match keymaker_core::broker::Transport::send(&transport, &prepared) {
        Ok(resp) => {
            // Filter the credential out of whatever came back.
            let redactor = keymaker_core::redact::Redactor::new(&[secret.expose()]);
            let leaked = redactor.detects(resp.body.as_bytes());
            let body = redactor.redact(resp.body.as_bytes());
            println!("{}", String::from_utf8_lossy(&body));
            if leaked {
                eprintln!("keymaker: the response echoed the credential; it has been masked");
            }
            std::process::exit(if (200..300).contains(&resp.status) {
                0
            } else {
                1
            });
        }
        Err(e) => die(e),
    }
}

fn cmd_list(args: &Args) {
    let manifest = load_manifest(&args.manifest);
    let catalog = load_catalog();

    println!("tasks");
    if manifest.task_names().is_empty() {
        println!("  (none)");
    }
    for t in manifest.task_names() {
        println!("  {}  →  {}", t, manifest.task(t).unwrap_or(""));
    }

    println!("\nendpoints");
    if catalog.names().is_empty() {
        println!("  (none)");
    }
    for e in catalog.names() {
        println!("  {}", e);
    }

    println!("\nreferences (env: {})", args.env);
    match manifest.refs_for(&args.env) {
        Ok(refs) if refs.is_empty() => println!("  (none)"),
        Ok(refs) => {
            for r in refs {
                println!("  {}", r);
            }
        }
        Err(_) => println!("  (no such environment)"),
    }
}

fn cmd_doctor(args: &Args) {
    let manifest = load_manifest(&args.manifest);
    let store = open_store();
    let refs = manifest.refs_for(&args.env).unwrap_or_else(|e| die(e));

    let mut missing = Vec::new();
    for r in &refs {
        if store.get(r).is_err() {
            missing.push(r.clone());
        }
    }
    println!("environment: {}", args.env);
    println!("references:  {}", refs.len());
    if missing.is_empty() {
        println!("all present");
    } else {
        println!("missing on this machine:");
        for m in &missing {
            println!("  {}", m);
        }
        std::process::exit(1);
    }
}

fn cmd_set(args: &Args) {
    let Some(reference) = args.rest.first() else {
        die("set needs a reference, e.g. stripe/sk_live")
    };
    let mut value = String::new();
    if std::io::stdin().is_terminal() {
        eprint!(
            "value for {} (input is not echoed by your terminal if piped): ",
            reference
        );
        std::io::stderr().flush().ok();
    }
    std::io::stdin()
        .read_to_string(&mut value)
        .unwrap_or_else(|e| die(format!("reading value: {}", e)));
    let value = value.trim_end_matches('\n').to_string();
    if value.is_empty() {
        die("refusing to store an empty value");
    }
    let mut store = open_store();
    store
        .set(reference, Secret::new(value))
        .unwrap_or_else(|e| die(e));
    println!("stored {}", reference);
}

fn cmd_rm(args: &Args) {
    let Some(reference) = args.rest.first() else {
        die("rm needs a reference")
    };
    let mut store = open_store();
    store.delete(reference).unwrap_or_else(|e| die(e));
    println!("removed {}", reference);
}

fn cmd_audit(args: &Args) {
    let path = config_dir().join("audit.jsonl");
    let clock = SystemClock;
    let log = match keymaker_core::audit::Log::open(&clock, &path) {
        Ok(l) => l,
        Err(e) => die(format!("{} ({})", e, path.display())),
    };

    if args.rest.iter().any(|a| a == "--verify") {
        match log.verify() {
            Ok(()) => println!(
                "audit chain intact: {} entries in {}",
                log.len(),
                path.display()
            ),
            Err(t) => die(format!("audit chain broken at {:?}", t)),
        }
        return;
    }
    if log.is_empty() {
        println!("(no audit entries yet; {} )", path.display());
        return;
    }
    for e in log.entries() {
        println!(
            "{:>5}  {}  {}",
            e.seq,
            e.at,
            serde_json::to_string(&e.event).unwrap_or_default()
        );
    }
}

fn cmd_serve(args: &Args) {
    use keymaker_core::broker::Broker;
    use keymaker_core::id::OsEntropy;
    use keymaker_core::server::{bind, default_socket_path, serve};

    let path = args
        .rest
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path);

    let clock = SystemClock;
    let entropy = OsEntropy;
    let store = open_store();
    let spawner = ProcessSpawner;
    let transport = keymaker_core::transport::HttpTransport::default();
    let manifest = load_manifest(&args.manifest);
    let catalog = load_catalog();

    let audit_path = config_dir().join("audit.jsonl");
    let audit = keymaker_core::audit::Log::open(&clock, &audit_path)
        // A broken chain is worth refusing to start for: appending to it would
        // bury the break, and the log exists precisely to be trusted.
        .unwrap_or_else(|e| die(format!("{} ({})", e, audit_path.display())));

    let mut broker = Broker::new(
        &clock,
        &entropy,
        store.as_ref(),
        &spawner,
        &transport,
        manifest,
        catalog,
        // Handles are short-lived by design: long enough for one tool-call,
        // not long enough to bank.
        60,
    )
    .with_audit(audit);

    let bound = bind(&path).unwrap_or_else(|e| die(e));
    eprintln!("keymaker: broker listening on {}", path.display());
    eprintln!("keymaker: audit trail at {}", audit_path.display());
    eprintln!("keymaker: {:?}", broker);
    if let Err(e) = serve(&mut broker, &bound) {
        die(e);
    }
}

fn cmd_mcp(args: &Args) {
    use keymaker_core::broker::Broker;
    use keymaker_core::id::OsEntropy;
    use keymaker_core::mcp::{serve_stdio, LocalDispatcher};

    // Prefer a real daemon: there the broker is a separate process, so a bug
    // in the MCP surface cannot reach the plaintext it holds.
    if let Some(client) = broker_client() {
        if let Err(e) = serve_stdio(client) {
            die(e);
        }
        return;
    }

    eprintln!("keymaker: no broker running; serving MCP in-process.");
    eprintln!("keymaker: start `keymaker serve` for the stronger arrangement.");

    let clock = SystemClock;
    let entropy = OsEntropy;
    let store = open_store();
    let spawner = ProcessSpawner;
    let transport = keymaker_core::transport::HttpTransport::default();
    let broker = Broker::new(
        &clock,
        &entropy,
        store.as_ref(),
        &spawner,
        &transport,
        load_manifest(&args.manifest),
        load_catalog(),
        60,
    );
    let peer = keymaker_core::peer::PeerIdentity {
        uid: keymaker_core::peer::current_uid(),
        gid: 0,
        pid: std::process::id() as i32,
        start_time: None,
        cwd: std::env::current_dir().ok(),
    };
    if let Err(e) = serve_stdio(LocalDispatcher::new(broker, peer)) {
        die(e);
    }
}

/// Connect to a running broker, if there is one.
fn broker_client() -> Option<keymaker_core::server::Client> {
    let path = keymaker_core::server::default_socket_path();
    if !path.exists() {
        return None;
    }
    keymaker_core::server::Client::connect(&path).ok()
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = argv.first().cloned() else {
        print!("{}", USAGE);
        return;
    };
    let args = parse(argv[1..].to_vec());

    match command.as_str() {
        "serve" => cmd_serve(&args),
        "mcp" => cmd_mcp(&args),
        "jail" => cmd_jail(&args),
        "run" => cmd_run(&args),
        "call" => cmd_call(&args),
        "list" | "ls" => cmd_list(&args),
        "doctor" => cmd_doctor(&args),
        "set" => cmd_set(&args),
        "rm" | "delete" => cmd_rm(&args),
        "audit" => cmd_audit(&args),
        "--help" | "-h" | "help" => print!("{}", USAGE),
        "--version" | "-V" | "version" => println!("keymaker {}", env!("CARGO_PKG_VERSION")),
        "get" | "export" | "reveal" | "cat" => {
            eprintln!("keymaker: there is no `{}`. That is the point.", command);
            eprintln!();
            eprintln!("A secret can be used, not read:");
            eprintln!("  keymaker run <task>        run something with it");
            eprintln!("  keymaker call <endpoint>   send a request with it");
            eprintln!();
            eprintln!("To read a value yourself, use the GUI.");
            std::process::exit(2);
        }
        other => {
            let _: Error;
            eprintln!("keymaker: unknown command `{}`", other);
            print!("{}", USAGE);
            std::process::exit(1);
        }
    }
}
