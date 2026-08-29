use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

mod keychain;

const DEFAULT_VAULT: &str = "seal";

fn index_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("seal");
    fs::create_dir_all(&path).ok();
    path.push("index.json");
    path
}

fn read_index() -> BTreeMap<String, Vec<String>> {
    let path = index_path();
    if !path.exists() {
        return BTreeMap::new();
    }
    let data = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&data).unwrap_or_default()
}

fn write_index(index: &BTreeMap<String, Vec<String>>) {
    let path = index_path();
    let data = serde_json::to_string_pretty(index).unwrap_or_default();
    fs::write(&path, data).ok();
}

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
    let mut index = read_index();
    let keys = index.entry(vault.clone()).or_default();
    if !keys.contains(&key) {
        keys.push(key.clone());
        keys.sort();
    }
    write_index(&index);
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
    let mut index = read_index();
    if let Some(keys) = index.get_mut(&vault) {
        keys.retain(|k| k != &key);
    }
    write_index(&index);
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

/// List every key in every vault, optionally filtered by `pattern`. The pattern
/// is tested against the namespaced `vault/key` and against the bare key, so
/// `seal list hard`, `seal list hardroad/db*` and `seal list *pass` all work.
fn cmd_list(pattern: Option<&str>) {
    let index = read_index();
    let mut found = false;
    for (vault, keys) in &index {
        for key in keys {
            let full = format!("{}/{}", vault, key);
            let hit = match pattern {
                None => true,
                Some(p) => matches(p, &full) || matches(p, key),
            };
            if hit {
                println!("{}", display_key(vault, key));
                found = true;
            }
        }
    }
    if !found && pattern.is_some() {
        std::process::exit(1);
    }
}

/// List one vault only (used when --vault/-v or SEAL_VAULT scopes the command).
fn cmd_list_vault(vault: &str, pattern: Option<&str>) {
    let index = read_index();
    let keys = index.get(vault).cloned().unwrap_or_default();
    let mut found = false;
    for key in &keys {
        if pattern.map_or(true, |p| matches(p, key)) {
            println!("{}", key);
            found = true;
        }
    }
    if !found && pattern.is_some() {
        std::process::exit(1);
    }
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
    eprintln!("  seal list                          List keys in every vault");
    eprintln!("  seal list <pattern>                Filter keys (substring or *? glob)");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --vault, -v <name>                 Default vault (overrides SEAL_VAULT env)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  seal set API_KEY \"sk-abc123\"");
    eprintln!("  seal set hardroad/db_pass \"hunter2\"");
    eprintln!("  seal get hardroad/db_pass");
    eprintln!("  seal list");
    eprintln!("  seal list hardroad");
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
