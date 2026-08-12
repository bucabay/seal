use keyring::Entry;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const DEFAULT_ACCOUNT: &str = "seal";

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

fn parse_key(raw: &str, default_account: &str) -> (String, String) {
    if let Some((account, key)) = raw.split_once('/') {
        (account.to_string(), key.to_string())
    } else {
        (default_account.to_string(), raw.to_string())
    }
}

fn cmd_set(key: &str, value: &str, account: &str) {
    let (account, key) = parse_key(key, account);
    let entry = match Entry::new("seal", &format!("{}:{}", account, key)) {
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
    let keys = index.entry(account.clone()).or_default();
    if !keys.contains(&key) {
        keys.push(key.clone());
        keys.sort();
    }
    write_index(&index);
    println!("Saved {}", if account == DEFAULT_ACCOUNT { key } else { format!("{}/{}", account, key) });
}

fn cmd_get(key: &str, account: &str) {
    let (account, key) = parse_key(key, account);
    let entry = match Entry::new("seal", &format!("{}:{}", account, key)) {
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

fn cmd_delete(key: &str, account: &str) {
    let (account, key) = parse_key(key, account);
    let entry = match Entry::new("seal", &format!("{}:{}", account, key)) {
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
    if let Some(keys) = index.get_mut(&account) {
        keys.retain(|k| k != &key);
    }
    write_index(&index);
    println!("Deleted {}", if account == DEFAULT_ACCOUNT { key } else { format!("{}/{}", account, key) });
}

fn cmd_list(account: &str) {
    let index = read_index();
    let keys = index.get(account).cloned().unwrap_or_default();
    for key in &keys {
        println!("{}", key);
    }
}

fn print_usage() {
    eprintln!("Seal — cross-platform secrets manager");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  seal set <key> <value>             Save a secret");
    eprintln!("  seal set ns/key value              Save under account=ns");
    eprintln!("  seal get <key>                     Retrieve a secret");
    eprintln!("  seal get ns/key                    Retrieve from account=ns");
    eprintln!("  seal delete <key>                  Delete a secret");
    eprintln!("  seal list [account]                List keys (default: seal)");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --account, -a <name>               Default account (overrides SEAL_ACCOUNT env)");
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

    let mut default_account = std::env::var("SEAL_ACCOUNT")
        .unwrap_or_else(|_| DEFAULT_ACCOUNT.to_string());

    // Parse --account / -a flag
    let mut i = 1;
    let mut filtered: Vec<String> = vec!["seal".to_string()];
    while i < args.len() {
        if args[i] == "--account" || args[i] == "-a" {
            if i + 1 < args.len() {
                default_account = args[i + 1].clone();
                i += 2;
            } else {
                eprintln!("Missing account name after {}", args[i]);
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
            cmd_set(&filtered[2], &filtered[3], &default_account);
        }
        "get" => {
            if filtered.len() < 3 {
                eprintln!("Usage: seal get <key>");
                std::process::exit(1);
            }
            cmd_get(&filtered[2], &default_account);
        }
        "delete" | "rm" => {
            if filtered.len() < 3 {
                eprintln!("Usage: seal delete <key>");
                std::process::exit(1);
            }
            cmd_delete(&filtered[2], &default_account);
        }
        "list" | "ls" => {
            let account = if filtered.len() > 2 {
                filtered[2].clone()
            } else {
                default_account.clone()
            };
            cmd_list(&account);
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
