use keyring::Entry;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

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
    let entry = match Entry::new("seal", &format!("{}:{}", vault, key)) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = entry.set_password(value) {
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
    let entry = match Entry::new("seal", &format!("{}:{}", vault, key)) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    match entry.get_password() {
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
    let entry = match Entry::new("seal", &format!("{}:{}", vault, key)) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = entry.delete_credential() {
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

fn cmd_list(vault: &str) {
    let index = read_index();
    let keys = index.get(vault).cloned().unwrap_or_default();
    for key in &keys {
        println!("{}", key);
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
    eprintln!("  seal list [vault]                  List keys (default: seal)");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --vault, -v <name>                 Default vault (overrides SEAL_VAULT env)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  seal set API_KEY \"sk-abc123\"");
    eprintln!("  seal set hardroad/db_pass \"hunter2\"");
    eprintln!("  seal get hardroad/db_pass");
    eprintln!("  seal list hardroad");
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

    let mut default_vault = std::env::var("SEAL_VAULT")
        .unwrap_or_else(|_| DEFAULT_VAULT.to_string());

    // Parse --vault / -v flag
    let mut i = 1;
    let mut filtered: Vec<String> = vec!["seal".to_string()];
    while i < args.len() {
        if args[i] == "--vault" || args[i] == "-v" {
            if i + 1 < args.len() {
                default_vault = args[i + 1].clone();
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
            let vault = if filtered.len() > 2 {
                filtered[2].clone()
            } else {
                default_vault.clone()
            };
            cmd_list(&vault);
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
