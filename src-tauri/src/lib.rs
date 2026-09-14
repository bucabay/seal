#![cfg(feature = "gui")]

use serde::Serialize;

mod index;
mod keychain;

use index::DEFAULT_VAULT;

#[derive(Serialize)]
struct SecretEntry {
    key: String,
    vault: String,
    /// Environment the value actually lives in. May differ from the one being
    /// viewed when the value is inherited.
    env: String,
    /// True when `env` is not the environment the caller asked for.
    inherited: bool,
}

#[derive(Serialize)]
struct EnvEntry {
    name: String,
    extends: Option<String>,
    /// Secrets stored directly in this environment, ignoring inheritance.
    own_count: usize,
}

fn or_default_vault(vault: Option<String>) -> String {
    vault.unwrap_or_else(|| DEFAULT_VAULT.to_string())
}

fn or_default_env(env: Option<String>) -> String {
    env.unwrap_or_else(|| index::DEFAULT_ENV.to_string())
}

#[tauri::command]
fn save_secret(
    key: String,
    value: String,
    vault: Option<String>,
    env: Option<String>,
) -> Result<String, String> {
    let vault = or_default_vault(vault);
    let env = or_default_env(env);
    keychain::set(&index::account(&vault, &env, &key), &value)?;
    index::add_key(&vault, &env, &key);

    Ok(format!("Saved {}:{}", vault, key))
}

#[tauri::command]
fn get_secret(key: String, vault: Option<String>, env: Option<String>) -> Result<String, String> {
    let vault = or_default_vault(vault);
    let env = or_default_env(env);
    // Walk the extends chain so the GUI reveals the value the app would get.
    let catalogue = index::load();
    let chain = catalogue
        .vault(&vault)
        .map(|v| v.chain(&env))
        .unwrap_or_else(|| vec![env.clone()]);
    for link in chain {
        if let Ok(value) = keychain::get(&index::account(&vault, &link, &key)) {
            return Ok(value);
        }
    }
    Err(format!("Not found: {} (env {})", key, env))
}

#[tauri::command]
fn delete_secret(
    key: String,
    vault: Option<String>,
    env: Option<String>,
) -> Result<String, String> {
    let vault = or_default_vault(vault);
    let env = or_default_env(env);
    // Only ever delete what this environment owns; an inherited value belongs
    // to the environment that defines it.
    keychain::delete(&index::account(&vault, &env, &key))?;
    index::remove_key(&vault, &env, &key);

    Ok(format!("Deleted {}:{}", vault, key))
}

#[tauri::command]
fn list_secrets(vault: Option<String>, env: Option<String>) -> Result<Vec<SecretEntry>, String> {
    let vault_name = or_default_vault(vault);
    let env = or_default_env(env);
    let catalogue = index::load();
    let Some(v) = catalogue.vault(&vault_name) else {
        return Ok(Vec::new());
    };
    Ok(v.resolved_keys(&env)
        .into_iter()
        .map(|(key, owner)| SecretEntry {
            key,
            vault: vault_name.clone(),
            inherited: owner != env,
            env: owner,
        })
        .collect())
}

#[tauri::command]
fn list_envs(vault: Option<String>) -> Result<Vec<EnvEntry>, String> {
    let vault_name = or_default_vault(vault);
    let catalogue = index::load();
    let Some(v) = catalogue.vault(&vault_name) else {
        return Ok(vec![EnvEntry {
            name: index::DEFAULT_ENV.to_string(),
            extends: None,
            own_count: 0,
        }]);
    };
    Ok(v.envs
        .iter()
        .map(|(name, cfg)| EnvEntry {
            name: name.clone(),
            extends: cfg.extends.clone(),
            own_count: v.own_keys(name).len(),
        })
        .collect())
}

#[tauri::command]
fn add_env(
    vault: Option<String>,
    name: String,
    extends: Option<String>,
) -> Result<Vec<EnvEntry>, String> {
    let vault_name = or_default_vault(vault);
    index::add_env(&vault_name, name.trim(), extends.as_deref())?;
    list_envs(Some(vault_name))
}

#[tauri::command]
fn delete_env(vault: Option<String>, name: String) -> Result<Vec<EnvEntry>, String> {
    let vault_name = or_default_vault(vault);
    index::remove_env(&vault_name, &name)?;
    list_envs(Some(vault_name))
}

#[tauri::command]
fn list_vaults() -> Result<Vec<String>, String> {
    let catalogue = index::load();
    let mut vaults: Vec<String> = catalogue.vaults.keys().cloned().collect();
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
    if vault.contains('/') || vault.contains(':') {
        return Err("Vault name cannot contain '/' or ':'".to_string());
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
            list_envs,
            add_env,
            delete_env,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
