mod index;
mod keychain;

use index::DEFAULT_VAULT;

fn parse_key(raw: &str, default_vault: &str) -> (String, String) {
    if let Some((vault, key)) = raw.split_once('/') {
        (vault.to_string(), key.to_string())
    } else {
        (default_vault.to_string(), raw.to_string())
    }
}

/// Where a `get` would find `key`, walking `env` and everything it extends.
///
/// The keychain is consulted at each link rather than the index, so a stale
/// index cannot make a secret that exists look missing.
fn resolve(index: &index::Index, vault: &str, env: &str, key: &str) -> Option<(String, String)> {
    let chain = index
        .vault(vault)
        .map(|v| v.chain(env))
        .unwrap_or_else(|| vec![env.to_string()]);
    for link in chain {
        if let Ok(value) = keychain::get(&index::account(vault, &link, key)) {
            return Some((link, value));
        }
    }
    None
}

/// How a secret is named back to the user: `vault/key`, bare in the default
/// vault, with the environment appended only when it is not the root.
fn display_ref(vault: &str, key: &str) -> String {
    display_key(vault, key)
}

fn cmd_set(key: &str, value: &str, vault: &str, env: &str) {
    let (vault, key) = parse_key(key, vault);
    if let Err(e) = keychain::set(&index::account(&vault, env, &key), value) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
    index::add_key(&vault, env, &key);
    println!(
        "Saved {}{}",
        display_ref(&vault, &key),
        env_suffix(env, Some(env))
    );
}

fn cmd_get(key: &str, vault: &str, env: &str) {
    let (vault, key) = parse_key(key, vault);
    let index = index::load();
    match resolve(&index, &vault, env, &key) {
        Some((_, value)) => println!("{}", value),
        None => {
            eprintln!("Not found: {} (env {})", key, env);
            std::process::exit(1);
        }
    }
}

fn cmd_delete(key: &str, vault: &str, env: &str) {
    let (vault, key) = parse_key(key, vault);
    // Delete only what this environment owns; an inherited value belongs to the
    // environment that defines it and must not vanish from under its siblings.
    if keychain::delete(&index::account(&vault, env, &key)).is_err() {
        let index = index::load();
        match resolve(&index, &vault, env, &key) {
            Some((owner, _)) => eprintln!(
                "Not found in '{}': {} is inherited from '{}'. Delete it there with -e {}.",
                env, key, owner, owner
            ),
            None => eprintln!("Not found: {} (env {})", key, env),
        }
        std::process::exit(1);
    }
    index::remove_key(&vault, env, &key);
    println!(
        "Deleted {}{}",
        display_ref(&vault, &key),
        env_suffix(env, Some(env))
    );
}

/// `@env` tag, omitted for the root environment so default-only users never see
/// environment noise.
///
/// `viewing` is the environment the user asked to see. When it is `None` no
/// environment was selected and the listing shows where each secret is stored,
/// so nothing is inherited from the viewer's point of view.
fn env_suffix(env: &str, viewing: Option<&str>) -> String {
    // Inherited only means something when the user picked an environment to
    // view; otherwise the listing is showing where each secret is stored.
    let inherited = viewing.is_some_and(|v| v != env);
    if inherited {
        format!("  @{} (inherited)", env)
    } else if env == index::DEFAULT_ENV {
        String::new()
    } else {
        format!("  @{}", env)
    }
}

/// Glob match supporting `*` (any run, including `/`) and `?` (one char).
/// Both sides are expected to be lowercased already.
fn glob_match(pattern: &[char], text: &[char]) -> bool {
    // Iterative backtracking: linear in the common case, no recursion depth risk.
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            resume = t;
            p += 1;
        } else if let Some(sp) = star {
            // Backtrack: let the last `*` swallow one more character.
            p = sp + 1;
            resume += 1;
            t = resume;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// A pattern matches a key if it globs (when it contains `*`/`?`) or appears
/// anywhere in it (plain substring). Matching is case-insensitive.
fn matches(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let candidate = candidate.to_lowercase();
    if pattern.contains('*') || pattern.contains('?') {
        let p: Vec<char> = pattern.chars().collect();
        let c: Vec<char> = candidate.chars().collect();
        glob_match(&p, &c)
    } else {
        candidate.contains(&pattern)
    }
}

fn display_key(vault: &str, key: &str) -> String {
    if vault == DEFAULT_VAULT {
        key.to_string()
    } else {
        format!("{}/{}", vault, key)
    }
}

/// A pattern that means "everything": absent, empty, or a bare `*`. `seal list`,
/// `seal list ''` and `seal list '*'` are the same command.
fn effective_pattern(pattern: Option<&str>) -> Option<&str> {
    pattern.filter(|p| !p.is_empty() && *p != "*" && *p != "*/*")
}

/// One listable secret: where it lives, and how it is addressed.
struct Row {
    reference: String,
    env: String,
}

fn print_rows(rows: &[Row], viewing: Option<&str>) {
    let width = rows.iter().map(|r| r.reference.len()).max().unwrap_or(0);
    for row in rows {
        let suffix = env_suffix(&row.env, viewing);
        if suffix.is_empty() {
            println!("{}", row.reference);
        } else {
            println!("{:width$}{}", row.reference, suffix, width = width);
        }
    }
}

/// Collect the rows a listing should show.
///
/// With an environment selected the result is the *effective* configuration for
/// it — its own keys plus everything inherited through the extends chain, each
/// tagged with the environment that actually holds it. Without one, every key
/// in every environment is listed as it is stored.
fn collect(index: &index::Index, vault_scope: Option<&str>, env: Option<&str>) -> Vec<Row> {
    let mut rows = Vec::new();
    for (vault_name, vault) in &index.vaults {
        if let Some(scope) = vault_scope {
            if vault_name != scope {
                continue;
            }
        }
        match env {
            Some(env) => {
                for (key, owner) in vault.resolved_keys(env) {
                    rows.push(Row {
                        reference: display_ref(vault_name, &key),
                        env: owner,
                    });
                }
            }
            None => {
                for (env_name, keys) in &vault.keys {
                    for key in keys {
                        rows.push(Row {
                            reference: display_ref(vault_name, key),
                            env: env_name.clone(),
                        });
                    }
                }
            }
        }
    }
    rows
}

/// List secrets, optionally narrowed by `pattern`, scoped to one vault, and
/// resolved through one environment.
///
/// A pattern that names a vault (`seal list hardroad`, `seal list hardroad/`)
/// lists that namespace. Anything else is matched against both the namespaced
/// `vault/key` and the bare key, so `seal list hardroad/db*` and
/// `seal list '*pass'` both work.
fn cmd_list(pattern: Option<&str>, vault_scope: Option<&str>, env: Option<&str>) {
    let index = index::load();
    let pattern = effective_pattern(pattern);

    // An explicit --vault/SEAL_VAULT scope, or a pattern that names a vault.
    // Exact namespace wins over substring matching, so `seal list seal` lists
    // the default vault rather than every key with "seal" in its name.
    let mut scope = vault_scope.and_then(|v| index.find_vault(v).cloned());
    let mut pattern_named_vault = false;
    if scope.is_none() {
        if let Some(name) = pattern {
            let name = name.strip_suffix('/').unwrap_or(name);
            if let Some(vault) = index.find_vault(name) {
                scope = Some(vault.clone());
                pattern_named_vault = true;
            }
        }
    }

    if let Some(env) = env {
        if let Some(v) = &scope {
            if let Some(vault) = index.vault(v) {
                if !vault.envs.contains_key(env) {
                    eprintln!("No such environment in '{}': {}", v, env);
                    std::process::exit(1);
                }
            }
        }
    }

    let rows = collect(&index, scope.as_deref(), env);
    let filter = if pattern_named_vault { None } else { pattern };
    let matched: Vec<Row> = rows
        .into_iter()
        .filter(|row| {
            filter.map_or(true, |p| {
                matches(p, &row.reference) || matches(p, row.reference.rsplit('/').next().unwrap_or(""))
            })
        })
        .collect();

    if !matched.is_empty() {
        print_rows(&matched, env);
        return;
    }
    // A named-but-empty vault or environment is a valid, successful listing of
    // nothing; a pattern that matched nothing is a failed lookup.
    if scope.is_some() {
        eprintln!("No secrets in '{}'", scope.as_deref().unwrap_or_default());
        return;
    }
    if pattern.is_some() {
        std::process::exit(1);
    }
    eprintln!("No secrets yet — save one with `seal set <key> <value>`");
}

/// Split `mailkite/prod` into an explicit vault and environment, falling back
/// to the scoped vault when the name carries no `/`.
fn parse_env_ref(raw: &str, default_vault: &str) -> (String, String) {
    parse_key(raw, default_vault)
}

fn cmd_env_list(vault_name: &str) {
    let index = index::load();
    let Some(vault) = index.find_vault(vault_name).and_then(|v| index.vault(v)) else {
        eprintln!("No such vault: {}", vault_name);
        std::process::exit(1);
    };
    let rows: Vec<(String, String, usize)> = vault
        .envs
        .iter()
        .map(|(name, cfg)| {
            let lineage = match &cfg.extends {
                Some(parent) => format!("extends {}", parent),
                None => "root".to_string(),
            };
            (name.clone(), lineage, vault.own_keys(name).len())
        })
        .collect();
    let name_w = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    let lineage_w = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
    for (name, lineage, count) in rows {
        println!(
            "{:name_w$}  {:lineage_w$}  {} secret{}",
            name,
            lineage,
            count,
            if count == 1 { "" } else { "s" },
            name_w = name_w,
            lineage_w = lineage_w
        );
    }
}

fn cmd_env_add(raw: &str, extends: Option<&str>, default_vault: &str) {
    let (vault, env) = parse_env_ref(raw, default_vault);
    match index::add_env(&vault, &env, extends) {
        Ok(()) => {
            let index = index::load();
            let chain = index
                .vault(&vault)
                .map(|v| v.chain(&env))
                .unwrap_or_default();
            println!("Environment {}/{} — {}", vault, env, chain.join(" -> "));
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_env_rm(raw: &str, default_vault: &str) {
    let (vault, env) = parse_env_ref(raw, default_vault);
    if let Err(e) = index::remove_env(&vault, &env) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
    println!("Removed environment {}/{}", vault, env);
}

fn print_usage() {
    eprintln!("Seal — cross-platform secrets manager");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  seal set <key> <value>             Save a secret");
    eprintln!("  seal set ns/key value              Save under vault=ns");
    eprintln!("  seal get <key>                     Retrieve a secret");
    eprintln!("  seal get ns/key                    Retrieve from vault=ns");
    eprintln!("  seal delete <key>                  Delete a secret");
    eprintln!("  seal list                          List every key in every vault");
    eprintln!("  seal list <ns>                     List one vault (namespace)");
    eprintln!("  seal list <pattern>                Filter keys (substring or *? glob)");
    eprintln!();
    eprintln!("Environments:");
    eprintln!("  seal env                           List environments and what they extend");
    eprintln!("  seal env add <name> [--extends p]  Declare an environment (defaults to");
    eprintln!("                                     extending `default`)");
    eprintln!("  seal env rm <name>                 Remove an empty environment");
    eprintln!();
    eprintln!("  Each vault has its own environments, rooted at `default`. Reading from");
    eprintln!("  an environment falls back through what it extends, so an environment");
    eprintln!("  only needs to store what it overrides.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --vault, -v <name>                 Vault (overrides SEAL_VAULT env)");
    eprintln!("  --env,   -e <name>                 Environment (overrides SEAL_ENV env)");
    eprintln!("  --version, -V                      Print the installed version");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  seal set API_KEY \"sk-abc123\"");
    eprintln!("  seal set hardroad/db_pass \"hunter2\"");
    eprintln!("  seal get hardroad/db_pass");
    eprintln!("  seal list                          # everything; same as `seal list '*'`");
    eprintln!("  seal list hardroad                 # everything in vault `hardroad`");
    eprintln!("  seal list hardroad/db*             # globbed within a vault");
    eprintln!("  seal list '*_key'");
    eprintln!();
    eprintln!("  seal env add prod -v mailkite              # prod extends default");
    eprintln!("  seal env add dev --extends prod -v mailkite");
    eprintln!("  seal set mailkite/db_url \"...\" -e prod     # override just this key");
    eprintln!("  seal get mailkite/api_key -e dev           # dev -> prod -> default");
    eprintln!("  seal list mailkite -e dev                  # effective config for dev");
    eprintln!();
    eprintln!("Backends: macOS Keychain | Linux Secret Service | Windows Credential Manager");
}

/// A flag taking one value, e.g. `-v hardroad`. Returns the value and how many
/// argv slots it consumed.
fn take_value(args: &[String], i: usize) -> String {
    match args.get(i + 1) {
        Some(v) => v.clone(),
        None => {
            eprintln!("Missing value after {}", args[i]);
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() == 1 {
        #[cfg(feature = "gui")]
        {
            seal_lib::run();
            return;
        }
        #[cfg(not(feature = "gui"))]
        {
            print_usage();
            return;
        }
    }

    let env_vault = std::env::var("SEAL_VAULT").ok();
    let mut vault_is_explicit = env_vault.is_some();
    let mut default_vault = env_vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());

    let env_name = std::env::var("SEAL_ENV").ok();
    let mut env_is_explicit = env_name.is_some();
    let mut environment = env_name.unwrap_or_else(|| index::DEFAULT_ENV.to_string());

    let mut extends: Option<String> = None;

    let mut i = 1;
    let mut filtered: Vec<String> = vec!["seal".to_string()];
    while i < args.len() {
        match args[i].as_str() {
            "--vault" | "-v" => {
                default_vault = take_value(&args, i);
                vault_is_explicit = true;
                i += 2;
            }
            "--env" | "-e" => {
                environment = take_value(&args, i);
                env_is_explicit = true;
                i += 2;
            }
            "--extends" => {
                extends = Some(take_value(&args, i));
                i += 2;
            }
            _ => {
                filtered.push(args[i].clone());
                i += 1;
            }
        }
    }

    if filtered.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    match filtered[1].as_str() {
        "set" | "save" => {
            if filtered.len() < 4 {
                eprintln!("Usage: seal set <key> <value>");
                std::process::exit(1);
            }
            cmd_set(&filtered[2], &filtered[3], &default_vault, &environment);
        }
        "get" => {
            if filtered.len() < 3 {
                eprintln!("Usage: seal get <key>");
                std::process::exit(1);
            }
            cmd_get(&filtered[2], &default_vault, &environment);
        }
        "delete" | "rm" => {
            if filtered.len() < 3 {
                eprintln!("Usage: seal delete <key>");
                std::process::exit(1);
            }
            cmd_delete(&filtered[2], &default_vault, &environment);
        }
        "list" | "ls" => {
            let pattern = filtered.get(2).map(|s| s.as_str());
            // Bare `seal list` shows every vault; a scope or argument narrows it.
            let scope = vault_is_explicit.then(|| default_vault.as_str());
            let env = env_is_explicit.then(|| environment.as_str());
            cmd_list(pattern, scope, env);
        }
        "env" => match filtered.get(2).map(|s| s.as_str()) {
            None | Some("list") | Some("ls") => cmd_env_list(&default_vault),
            Some("add") | Some("set") => {
                let Some(name) = filtered.get(3) else {
                    eprintln!("Usage: seal env add <name> [--extends <parent>]");
                    std::process::exit(1);
                };
                cmd_env_add(name, extends.as_deref(), &default_vault);
            }
            Some("rm") | Some("remove") | Some("delete") => {
                let Some(name) = filtered.get(3) else {
                    eprintln!("Usage: seal env rm <name>");
                    std::process::exit(1);
                };
                cmd_env_rm(name, &default_vault);
            }
            Some(other) => {
                eprintln!("Unknown env command: {}", other);
                eprintln!("Usage: seal env [list | add <name> [--extends <p>] | rm <name>]");
                std::process::exit(1);
            }
        },
        "--version" | "-V" | "version" => {
            println!("seal {}", env!("CARGO_PKG_VERSION"));
        }
        "--help" | "-h" | "help" => {
            print_usage();
        }
        _ => {
            eprintln!("Unknown command: {}", filtered[1]);
            print_usage();
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_empty_and_star_all_mean_everything() {
        assert_eq!(effective_pattern(None), None);
        assert_eq!(effective_pattern(Some("")), None);
        assert_eq!(effective_pattern(Some("*")), None);
        assert_eq!(effective_pattern(Some("*/*")), None);
        assert_eq!(effective_pattern(Some("hardroad")), Some("hardroad"));
    }

    #[test]
    fn plain_patterns_match_as_substrings_case_insensitively() {
        assert!(matches("mail", "mailkite/openai-secret"));
        assert!(matches("MAIL", "mailkite/openai-secret"));
        assert!(matches("kite/openai", "mailkite/openai-secret"));
        assert!(!matches("stripe", "mailkite/openai-secret"));
    }

    #[test]
    fn globs_anchor_to_the_whole_candidate_and_cross_slashes() {
        assert!(matches("*_key", "hardroad/api_key"));
        assert!(matches("hardroad/db*", "hardroad/db_pass"));
        assert!(matches("hard*/*pass", "hardroad/db_pass"));
        assert!(matches("db_pas?", "db_pass"));
        assert!(!matches("hardroad/db*", "other/db_pass"));
        assert!(!matches("db_pas?", "db_passs"));
    }

    #[test]
    fn display_strips_only_the_default_vault() {
        assert_eq!(display_key(DEFAULT_VAULT, "token"), "token");
        assert_eq!(display_key("hardroad", "token"), "hardroad/token");
    }

    #[test]
    fn namespaced_keys_split_on_the_first_slash() {
        assert_eq!(
            parse_key("hardroad/db_pass", DEFAULT_VAULT),
            ("hardroad".into(), "db_pass".into())
        );
        assert_eq!(
            parse_key("token", "gabe"),
            ("gabe".into(), "token".into())
        );
    }
}
