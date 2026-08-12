use keyring::Entry;
use serde::Serialize;
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

#[derive(Serialize)]
struct SecretEntry {
    key: String,
    account: String,
}

#[tauri::command]
fn save_secret(key: String, value: String, account: Option<String>) -> Result<String, String> {
    let account = account.unwrap_or_else(|| DEFAULT_ACCOUNT.to_string());
    let entry = Entry::new("seal", &format!("{}:{}", account, key))
        .map_err(|e| format!("Keyring error: {}", e))?;
    entry
        .set_password(&value)
        .map_err(|e| format!("Keyring error: {}", e))?;

    let mut index = read_index();
    let keys = index.entry(account.clone()).or_default();
    if !keys.contains(&key) {
        keys.push(key.clone());
        keys.sort();
    }
    write_index(&index);

    Ok(format!("Saved {}:{}", account, key))
}

#[tauri::command]
fn get_secret(key: String, account: Option<String>) -> Result<String, String> {
    let account = account.unwrap_or_else(|| DEFAULT_ACCOUNT.to_string());
    let entry = Entry::new("seal", &format!("{}:{}", account, key))
        .map_err(|e| format!("Keyring error: {}", e))?;
    entry
        .get_password()
        .map_err(|e| format!("Not found: {}:{} ({})", account, key, e))
}

#[tauri::command]
fn delete_secret(key: String, account: Option<String>) -> Result<String, String> {
    let account = account.unwrap_or_else(|| DEFAULT_ACCOUNT.to_string());
    let entry = Entry::new("seal", &format!("{}:{}", account, key))
        .map_err(|e| format!("Keyring error: {}", e))?;
    entry
        .delete_credential()
        .map_err(|e| format!("Not found: {}:{} ({})", account, key, e))?;

    let mut index = read_index();
    if let Some(keys) = index.get_mut(&account) {
        keys.retain(|k| k != &key);
    }
    write_index(&index);

    Ok(format!("Deleted {}:{}", account, key))
}

#[tauri::command]
fn list_secrets(account: Option<String>) -> Result<Vec<SecretEntry>, String> {
    let account = account.unwrap_or_else(|| DEFAULT_ACCOUNT.to_string());
    let index = read_index();
    let keys = index.get(&account).cloned().unwrap_or_default();
    Ok(keys
        .into_iter()
        .map(|key| SecretEntry {
            key,
            account: account.clone(),
        })
        .collect())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
