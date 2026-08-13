#![cfg(feature = "gui")]

use keyring::Entry;
use serde::Serialize;
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

#[derive(Serialize)]
struct SecretEntry {
    key: String,
    vault: String,
}

#[tauri::command]
fn save_secret(key: String, value: String, vault: Option<String>) -> Result<String, String> {
    let vault = vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());
    let entry = Entry::new("seal", &format!("{}:{}", vault, key))
        .map_err(|e| format!("Keyring error: {}", e))?;
    entry
        .set_password(&value)
        .map_err(|e| format!("Keyring error: {}", e))?;

    let mut index = read_index();
    let keys = index.entry(vault.clone()).or_default();
    if !keys.contains(&key) {
        keys.push(key.clone());
        keys.sort();
    }
    write_index(&index);

    Ok(format!("Saved {}:{}", vault, key))
}

#[tauri::command]
fn get_secret(key: String, vault: Option<String>) -> Result<String, String> {
    let vault = vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());
    let entry = Entry::new("seal", &format!("{}:{}", vault, key))
        .map_err(|e| format!("Keyring error: {}", e))?;
    entry
        .get_password()
        .map_err(|e| format!("Not found: {}:{} ({})", vault, key, e))
}

#[tauri::command]
fn delete_secret(key: String, vault: Option<String>) -> Result<String, String> {
    let vault = vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());
    let entry = Entry::new("seal", &format!("{}:{}", vault, key))
        .map_err(|e| format!("Keyring error: {}", e))?;
    entry
        .delete_credential()
        .map_err(|e| format!("Not found: {}:{} ({})", vault, key, e))?;

    let mut index = read_index();
    if let Some(keys) = index.get_mut(&vault) {
        keys.retain(|k| k != &key);
    }
    write_index(&index);

    Ok(format!("Deleted {}:{}", vault, key))
}

#[tauri::command]
fn list_secrets(vault: Option<String>) -> Result<Vec<SecretEntry>, String> {
    let vault = vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());
    let index = read_index();
    let keys = index.get(&vault).cloned().unwrap_or_default();
    Ok(keys
        .into_iter()
        .map(|key| SecretEntry {
            key,
            vault: vault.clone(),
        })
        .collect())
}

#[tauri::command]
fn list_vaults() -> Result<Vec<String>, String> {
    let index = read_index();
    let mut vaults: Vec<String> = index.keys().cloned().collect();
    vaults.sort();
    if !vaults.contains(&DEFAULT_VAULT.to_string()) {
        vaults.insert(0, DEFAULT_VAULT.to_string());
    }
    Ok(vaults)
}

#[tauri::command]
fn add_vault(vault: String) -> Result<Vec<String>, String> {
    let vault = vault.trim().to_string();
    if vault.is_empty() {
        return Err("Vault name cannot be empty".to_string());
    }
    if vault.contains('/') {
        return Err("Vault name cannot contain '/'".to_string());
    }
    let mut index = read_index();
    index.entry(vault.clone()).or_default();
    write_index(&index);
    list_vaults()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            save_secret,
            get_secret,
            delete_secret,
            list_secrets,
            list_vaults,
            add_vault,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
