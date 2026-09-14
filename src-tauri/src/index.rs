//! The `vault -> keys` catalogue behind `seal list` and the GUI's vault picker.
//!
//! Keychain APIs are keyed lookups, not directories, so Seal has always kept a
//! local index of the names it has written. That index is a cache, never the
//! truth: it goes missing on a fresh machine, and drifts whenever a secret is
//! written by another build or removed with `security`/`seahorse`. Where the
//! platform *can* enumerate its keychain we therefore read the names back from
//! it and rewrite the cache, so a stale index can no longer hide a secret.

// Compiled into both the CLI binary and the GUI library, each of which uses a
// different subset of these helpers.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::keychain;

pub const DEFAULT_VAULT: &str = "seal";

/// Vault name -> sorted key names. Values never appear here.
pub type Index = BTreeMap<String, Vec<String>>;

pub fn index_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("seal");
    fs::create_dir_all(&path).ok();
    path.push("index.json");
    path
}

fn read_index() -> Index {
    let path = index_path();
    if !path.exists() {
        return Index::new();
    }
    let data = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&data).unwrap_or_default()
}

fn write_index(index: &Index) {
    let path = index_path();
    let data = serde_json::to_string_pretty(index).unwrap_or_default();
    fs::write(&path, data).ok();
}

/// Every vault and key Seal can see on this machine.
///
/// On platforms that can enumerate the keychain the result is derived from it
/// and the on-disk cache is refreshed as a side effect; elsewhere the cache is
/// all there is.
pub fn load() -> Index {
    let cached = read_index();

    let Some(accounts) = keychain::list_accounts() else {
        return cached;
    };
    // A locked or unreadable keychain enumerates as empty. Never let that
    // erase a cache that still has names in it.
    if accounts.is_empty() && !cached.is_empty() {
        return cached;
    }

    let mut live = Index::new();
    for account in accounts {
        // Accounts are stored as "{vault}:{key}"; a key may itself contain ':'.
        let Some((vault, key)) = account.split_once(':') else {
            continue;
        };
        if vault.is_empty() || key.is_empty() {
            continue;
        }
        live.entry(vault.to_string()).or_default().push(key.to_string());
    }
    // Vaults created in the GUI but never filled exist only in the cache.
    for (vault, keys) in &cached {
        if keys.is_empty() {
            live.entry(vault.clone()).or_default();
        }
    }
    for keys in live.values_mut() {
        keys.sort();
        keys.dedup();
    }

    if live != cached {
        write_index(&live);
    }
    live
}

/// Record a freshly written key.
pub fn add(vault: &str, key: &str) {
    let mut index = load();
    let keys = index.entry(vault.to_string()).or_default();
    if !keys.iter().any(|k| k == key) {
        keys.push(key.to_string());
        keys.sort();
    }
    write_index(&index);
}

/// Forget a deleted key. The vault itself is kept so it stays selectable.
pub fn remove(vault: &str, key: &str) {
    let mut index = load();
    if let Some(keys) = index.get_mut(vault) {
        keys.retain(|k| k != key);
    }
    write_index(&index);
}

/// Register an empty vault so it survives until something is stored in it.
pub fn add_vault(vault: &str) {
    let mut index = load();
    index.entry(vault.to_string()).or_default();
    write_index(&index);
}

/// Resolve a vault name case-insensitively, as `seal list <ns>` accepts it.
pub fn find_vault<'a>(index: &'a Index, name: &str) -> Option<&'a String> {
    index
        .keys()
        .find(|v| v.as_str() == name)
        .or_else(|| index.keys().find(|v| v.eq_ignore_ascii_case(name)))
}
