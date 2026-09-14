#![cfg(feature = "gui")]

use serde::Serialize;

mod index;
mod keychain;

use index::DEFAULT_VAULT;

#[derive(Serialize)]
struct SecretEntry {
    key: String,
    vault: String,
}

#[tauri::command]
fn save_secret(key: String, value: String, vault: Option<String>) -> Result<String, String> {
    let vault = vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());
    let full_key = format!("{}:{}", vault, key);
    keychain::set(&full_key, &value)?;
    index::add(&vault, &key);

    Ok(format!("Saved {}:{}", vault, key))
}

#[tauri::command]
fn get_secret(key: String, vault: Option<String>) -> Result<String, String> {
    let vault = vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());
    let full_key = format!("{}:{}", vault, key);
    keychain::get(&full_key)
}

#[tauri::command]
fn delete_secret(key: String, vault: Option<String>) -> Result<String, String> {
    let vault = vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());
    let full_key = format!("{}:{}", vault, key);
    keychain::delete(&full_key)?;
    index::remove(&vault, &key);

    Ok(format!("Deleted {}:{}", vault, key))
}

#[tauri::command]
fn list_secrets(vault: Option<String>) -> Result<Vec<SecretEntry>, String> {
    let vault = vault.unwrap_or_else(|| DEFAULT_VAULT.to_string());
    let index = index::load();
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
    let index = index::load();
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
    index::add_vault(&vault);
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
