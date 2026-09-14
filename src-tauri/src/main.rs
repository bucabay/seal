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

fn cmd_set(key: &str, value: &str, vault: &str) {
    let (vault, key) = parse_key(key, vault);
    let full_key = format!("{}:{}", vault, key);
    if let Err(e) = keychain::set(&full_key, value) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
    index::add(&vault, &key);
    println!("Saved {}", if vault == DEFAULT_VAULT { key } else { format!("{}/{}", vault, key) });
}

fn cmd_get(key: &str, vault: &str) {
    let (vault, key) = parse_key(key, vault);
    let full_key = format!("{}:{}", vault, key);
    match keychain::get(&full_key) {
        Ok(value) => println!("{}", value),
        Err(e) => {
            eprintln!("Not found: {}", key);
            let _ = e;
            std::process::exit(1);
        }
    }
}

fn cmd_delete(key: &str, vault: &str) {
    let (vault, key) = parse_key(key, vault);
    let full_key = format!("{}:{}", vault, key);
    if let Err(e) = keychain::delete(&full_key) {
        eprintln!("Not found: {}", key);
        let _ = e;
        std::process::exit(1);
    }
    index::remove(&vault, &key);
    println!("Deleted {}", if vault == DEFAULT_VAULT { key } else { format!("{}/{}", vault, key) });
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

fn print_entries(vault: &str, keys: &[String], bare: bool) -> bool {
    for key in keys {
        println!("{}", if bare { key.clone() } else { display_key(vault, key) });
    }
    !keys.is_empty()
}

/// List every key in every vault, optionally narrowed by `pattern`.
///
/// A pattern that names a vault (`seal list hardroad`, `seal list hardroad/`)
/// lists that namespace. Anything else is matched against both the namespaced
/// `vault/key` and the bare key, so `seal list hardroad/db*` and
/// `seal list '*pass'` both work.
fn cmd_list(pattern: Option<&str>) {
    let index = index::load();
    let pattern = effective_pattern(pattern);

    // Exact namespace wins over substring matching, so `seal list seal` lists
    // the default vault rather than every key with "seal" in its name.
    if let Some(name) = pattern {
        let name = name.strip_suffix('/').unwrap_or(name);
        if let Some(vault) = index::find_vault(&index, name) {
            if !print_entries(vault, &index[vault], false) {
                eprintln!("Vault '{}' has no secrets", vault);
            }
            return;
        }
    }

    let mut found = false;
    for (vault, keys) in &index {
        for key in keys {
            let full = format!("{}/{}", vault, key);
            if pattern.map_or(true, |p| matches(p, &full) || matches(p, key)) {
                println!("{}", display_key(vault, key));
                found = true;
            }
        }
    }

    if !found {
        if pattern.is_some() {
            std::process::exit(1);
        }
        eprintln!("No secrets yet — save one with `seal set <key> <value>`");
    }
}

/// List one vault only (used when --vault/-v or SEAL_VAULT scopes the command).
/// Keys print bare here because the scope already names the vault.
fn cmd_list_vault(vault: &str, pattern: Option<&str>) {
    let index = index::load();
    let pattern = effective_pattern(pattern);
    let keys = index::find_vault(&index, vault)
        .map(|v| index[v].clone())
        .unwrap_or_default();

    let matched: Vec<String> = keys
        .into_iter()
        .filter(|key| pattern.map_or(true, |p| matches(p, key)))
        .collect();

    if print_entries(vault, &matched, true) {
        return;
    }
    if pattern.is_some() {
        std::process::exit(1);
    }
    eprintln!("Vault '{}' has no secrets (scope set by --vault/SEAL_VAULT)", vault);
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
    eprintln!("Options:");
    eprintln!("  --vault, -v <name>                 Default vault (overrides SEAL_VAULT env)");
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
    eprintln!("Backends: macOS Keychain | Linux Secret Service | Windows Credential Manager");
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

    // Parse --vault / -v flag
    let mut i = 1;
    let mut filtered: Vec<String> = vec!["seal".to_string()];
    while i < args.len() {
        if args[i] == "--vault" || args[i] == "-v" {
            if i + 1 < args.len() {
                default_vault = args[i + 1].clone();
                vault_is_explicit = true;
                i += 2;
            } else {
                eprintln!("Missing vault name after {}", args[i]);
                std::process::exit(1);
            }
        } else {
            filtered.push(args[i].clone());
            i += 1;
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
            cmd_set(&filtered[2], &filtered[3], &default_vault);
        }
        "get" => {
            if filtered.len() < 3 {
                eprintln!("Usage: seal get <key>");
                std::process::exit(1);
            }
            cmd_get(&filtered[2], &default_vault);
        }
        "delete" | "rm" => {
            if filtered.len() < 3 {
                eprintln!("Usage: seal delete <key>");
                std::process::exit(1);
            }
            cmd_delete(&filtered[2], &default_vault);
        }
        "list" | "ls" => {
            let pattern = filtered.get(2).map(|s| s.as_str());
            if vault_is_explicit {
                cmd_list_vault(&default_vault, pattern);
            } else {
                // Bare `seal list` shows every vault; an argument filters it.
                cmd_list(pattern);
            }
        }
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
