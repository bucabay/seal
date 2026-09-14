//! The vault / environment catalogue behind `seal list`, `seal env` and the
//! GUI's pickers.
//!
//! Two kinds of state live here and they are treated very differently:
//!
//! - **Key names** are *derived*. Where the platform can enumerate its keychain
//!   (macOS) they are read back from it on every load, so a missing or stale
//!   file cannot hide a secret. See [`crate::keychain::list_accounts`].
//! - **The environment graph** — which environments exist and what each one
//!   extends — is *authoritative*. It cannot be derived from key names (an
//!   environment may be declared before it holds anything, and `extends` is
//!   pure configuration), so it is stored and preserved across every rebuild.
//!
//! Neither kind ever contains a secret value.

// Compiled into both the CLI binary and the GUI library, each of which uses a
// different subset of these helpers.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::keychain;

pub const DEFAULT_VAULT: &str = "seal";
pub const DEFAULT_ENV: &str = "default";

/// Bumped when the on-disk shape changes. v1 was a bare `{vault: [keys]}` map.
const SCHEMA_VERSION: u32 = 2;

/// One environment's configuration. `default` is the root and extends nothing.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvConfig {
    /// Environment consulted when a key is not present in this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Vault {
    /// Declared environments. Always contains `default`.
    #[serde(default)]
    pub envs: BTreeMap<String, EnvConfig>,
    /// Environment -> sorted key names. Derived from the keychain where the
    /// platform allows it.
    #[serde(default)]
    pub keys: BTreeMap<String, Vec<String>>,
}

impl Vault {
    /// Keys stored *directly* in `env`, ignoring inheritance.
    pub fn own_keys(&self, env: &str) -> &[String] {
        self.keys.get(env).map(|k| k.as_slice()).unwrap_or(&[])
    }

    /// Declare `env` if it is new, as an overlay on the root rather than an
    /// island.
    ///
    /// A key written straight into a fresh environment (`seal set -e dev ...`)
    /// must inherit exactly like one declared with `seal env add`, so both
    /// paths default to extending `default`. An environment that already
    /// exists keeps whatever it extends.
    pub fn declare_env(&mut self, env: &str) {
        if self.envs.contains_key(env) {
            return;
        }
        let extends = (env != DEFAULT_ENV).then(|| DEFAULT_ENV.to_string());
        self.envs.insert(env.to_string(), EnvConfig { extends });
    }

    /// `env`, then what it extends, nearest first.
    ///
    /// A malformed graph must never hang the CLI, so an `extends` pointing at
    /// an unknown environment simply ends the walk, and a cycle is cut the
    /// moment it repeats.
    pub fn chain(&self, env: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = Some(env.to_string());
        while let Some(name) = cursor {
            if !seen.insert(name.clone()) {
                break;
            }
            let parent = self.envs.get(&name).and_then(|e| e.extends.clone());
            chain.push(name);
            cursor = parent;
        }
        chain
    }

    /// Would pointing `child` at `parent` close a loop?
    pub fn would_cycle(&self, child: &str, parent: &str) -> bool {
        child == parent || self.chain(parent).iter().any(|e| e == child)
    }

    /// The effective key set for `env`: every key reachable through the chain,
    /// each paired with the environment it actually lives in.
    pub fn resolved_keys(&self, env: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut seen = BTreeSet::new();
        // Nearest environment wins, so the first sighting of a key is the one
        // that would be returned by `get`.
        for link in self.chain(env) {
            for key in self.own_keys(&link) {
                if seen.insert(key.clone()) {
                    out.push((key.clone(), link.clone()));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub version: u32,
    #[serde(default)]
    pub vaults: BTreeMap<String, Vault>,
}

impl Default for Index {
    fn default() -> Self {
        Index {
            version: SCHEMA_VERSION,
            vaults: BTreeMap::new(),
        }
    }
}

impl Index {
    pub fn vault(&self, name: &str) -> Option<&Vault> {
        self.vaults.get(name)
    }

    /// Resolve a vault name case-insensitively, as the CLI accepts it.
    pub fn find_vault(&self, name: &str) -> Option<&String> {
        self.vaults
            .keys()
            .find(|v| v.as_str() == name)
            .or_else(|| self.vaults.keys().find(|v| v.eq_ignore_ascii_case(name)))
    }

    /// Ensure a vault exists and always has a `default` environment.
    pub fn vault_mut(&mut self, name: &str) -> &mut Vault {
        let vault = self.vaults.entry(name.to_string()).or_default();
        vault.declare_env(DEFAULT_ENV);
        vault
    }
}

// ---------------------------------------------------------------- addressing

/// Keychain account name for a (vault, env, key).
///
/// The default environment keeps the pre-0.2 `vault:key` form, so every secret
/// written before environments existed — and anything written by an older
/// binary — resolves unchanged with no migration step.
pub fn account(vault: &str, env: &str, key: &str) -> String {
    if env == DEFAULT_ENV {
        format!("{}:{}", vault, key)
    } else {
        format!("{}/{}:{}", vault, env, key)
    }
}

/// Inverse of [`account`].
///
/// Only the portion *before* the first `:` is inspected for the env separator,
/// so keys may still contain `:` and `/` exactly as they could before.
pub fn parse_account(account: &str) -> Option<(String, String, String)> {
    let (scope, key) = account.split_once(':')?;
    if scope.is_empty() || key.is_empty() {
        return None;
    }
    match scope.split_once('/') {
        None => Some((scope.into(), DEFAULT_ENV.into(), key.into())),
        Some((vault, env)) if !vault.is_empty() && !env.is_empty() && !env.contains('/') => {
            Some((vault.into(), env.into(), key.into()))
        }
        Some(_) => None,
    }
}

// ------------------------------------------------------------------- storage

pub fn index_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("seal");
    fs::create_dir_all(&path).ok();
    path.push("index.json");
    path
}

/// Parse either schema. v1 was `{"vault": ["key", ...]}` with no environments;
/// every key in it belongs to `default`.
pub fn parse(data: &str) -> Index {
    if let Ok(index) = serde_json::from_str::<Index>(data) {
        if index.version >= 2 {
            return index;
        }
    }
    let legacy: BTreeMap<String, Vec<String>> = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Index::default(),
    };
    let mut index = Index::default();
    for (vault_name, keys) in legacy {
        let vault = index.vault_mut(&vault_name);
        if !keys.is_empty() {
            vault.keys.insert(DEFAULT_ENV.to_string(), keys);
        }
    }
    index
}

fn read_stored() -> Index {
    let path = index_path();
    if !path.exists() {
        return Index::default();
    }
    parse(&fs::read_to_string(&path).unwrap_or_default())
}

fn write(index: &Index) {
    let data = serde_json::to_string_pretty(index).unwrap_or_default();
    fs::write(index_path(), data).ok();
}

/// Everything Seal can see on this machine.
///
/// Key names come from the keychain where it can be enumerated; the
/// environment graph always comes from the stored file.
pub fn load() -> Index {
    let stored = read_stored();

    let Some(accounts) = keychain::list_accounts() else {
        return stored;
    };
    // A locked or unreadable keychain enumerates as empty. Never let that erase
    // a file that still has names in it.
    if accounts.is_empty() && !stored.vaults.is_empty() {
        return stored;
    }

    let mut live = Index::default();
    for raw in accounts {
        let Some((vault_name, env, key)) = parse_account(&raw) else {
            continue;
        };
        let vault = live.vault_mut(&vault_name);
        vault.declare_env(&env);
        vault.keys.entry(env).or_default().push(key);
    }

    // Carry over configuration that key names cannot express: declared-but-empty
    // vaults and environments, and every `extends` edge.
    for (vault_name, stored_vault) in &stored.vaults {
        let vault = live.vault_mut(vault_name);
        for (env_name, cfg) in &stored_vault.envs {
            vault.declare_env(env_name);
            // Only an explicit edge is carried over. A stored `None` on a
            // non-root environment is the pre-fix shape, where implicitly
            // created environments were written as islands; leaving it out
            // lets `declare_env` heal them into overlays on the root.
            if cfg.extends.is_some() {
                if let Some(entry) = vault.envs.get_mut(env_name) {
                    entry.extends = cfg.extends.clone();
                }
            }
        }
    }

    for vault in live.vaults.values_mut() {
        for keys in vault.keys.values_mut() {
            keys.sort();
            keys.dedup();
        }
    }

    if live != stored {
        write(&live);
    }
    live
}

// ------------------------------------------------------------------- mutators

/// Record a freshly written key.
pub fn add_key(vault: &str, env: &str, key: &str) {
    let mut index = load();
    let v = index.vault_mut(vault);
    v.declare_env(env);
    let keys = v.keys.entry(env.to_string()).or_default();
    if !keys.iter().any(|k| k == key) {
        keys.push(key.to_string());
        keys.sort();
    }
    write(&index);
}

/// Forget a deleted key. The vault and environment are kept so they stay
/// selectable.
pub fn remove_key(vault: &str, env: &str, key: &str) {
    let mut index = load();
    if let Some(v) = index.vaults.get_mut(vault) {
        if let Some(keys) = v.keys.get_mut(env) {
            keys.retain(|k| k != key);
        }
    }
    write(&index);
}

/// Register an empty vault so it survives until something is stored in it.
pub fn add_vault(vault: &str) {
    let mut index = load();
    index.vault_mut(vault);
    write(&index);
}

/// Declare an environment, or repoint an existing one.
pub fn add_env(vault: &str, env: &str, extends: Option<&str>) -> Result<(), String> {
    if env.is_empty() {
        return Err("Environment name cannot be empty".into());
    }
    if env.contains('/') || env.contains(':') {
        return Err("Environment name cannot contain '/' or ':'".into());
    }
    if env == DEFAULT_ENV && extends.is_some() {
        return Err(format!("'{}' is the root environment and extends nothing", DEFAULT_ENV));
    }

    let mut index = load();
    let v = index.vault_mut(vault);

    // Default to extending the root, which is what makes a new environment an
    // overlay rather than an empty island.
    let parent = match env {
        DEFAULT_ENV => None,
        _ => Some(extends.unwrap_or(DEFAULT_ENV).to_string()),
    };
    if let Some(parent) = &parent {
        if !v.envs.contains_key(parent) {
            return Err(format!("No such environment: {}", parent));
        }
        if v.would_cycle(env, parent) {
            return Err(format!(
                "'{}' extending '{}' would form a cycle: {}",
                env,
                parent,
                v.chain(parent).join(" -> ")
            ));
        }
    }

    v.envs.insert(env.to_string(), EnvConfig { extends: parent });
    write(&index);
    Ok(())
}

/// Remove an environment declaration. Refuses while it still holds keys or
/// while another environment extends it.
pub fn remove_env(vault: &str, env: &str) -> Result<(), String> {
    if env == DEFAULT_ENV {
        return Err(format!("Cannot remove the '{}' environment", DEFAULT_ENV));
    }
    let mut index = load();
    let Some(v) = index.vaults.get_mut(vault) else {
        return Err(format!("No such vault: {}", vault));
    };
    if !v.envs.contains_key(env) {
        return Err(format!("No such environment: {}", env));
    }
    let held = v.own_keys(env).len();
    if held > 0 {
        return Err(format!(
            "'{}' still holds {} secret{} — delete them first",
            env,
            held,
            if held == 1 { "" } else { "s" }
        ));
    }
    let dependents: Vec<String> = v
        .envs
        .iter()
        .filter(|(_, cfg)| cfg.extends.as_deref() == Some(env))
        .map(|(name, _)| name.clone())
        .collect();
    if !dependents.is_empty() {
        return Err(format!(
            "'{}' is extended by {} — repoint {} first",
            env,
            dependents.join(", "),
            if dependents.len() == 1 { "it" } else { "them" }
        ));
    }
    v.envs.remove(env);
    v.keys.remove(env);
    write(&index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_with_chain() -> Vault {
        let mut v = Vault::default();
        v.envs.insert(DEFAULT_ENV.into(), EnvConfig { extends: None });
        v.envs.insert("prod".into(), EnvConfig { extends: Some(DEFAULT_ENV.into()) });
        v.envs.insert("dev".into(), EnvConfig { extends: Some("prod".into()) });
        v.keys.insert(DEFAULT_ENV.into(), vec!["api_key".into(), "db_url".into(), "log_level".into()]);
        v.keys.insert("prod".into(), vec!["db_url".into()]);
        v.keys.insert("dev".into(), vec!["db_url".into()]);
        v
    }

    #[test]
    fn default_env_keeps_the_legacy_account_form() {
        assert_eq!(account("mailkite", DEFAULT_ENV, "db_url"), "mailkite:db_url");
        assert_eq!(account("mailkite", "prod", "db_url"), "mailkite/prod:db_url");
    }

    #[test]
    fn accounts_round_trip() {
        for (vault, env, key) in [
            ("mailkite", DEFAULT_ENV, "db_url"),
            ("mailkite", "prod", "db_url"),
            // Keys could always contain ':' and '/'; environments must not
            // change that, since only the text before the first ':' is scoped.
            ("hardroad", "dev", "aws:secret"),
            ("hardroad", DEFAULT_ENV, "path/to/thing"),
            ("Gabe", DEFAULT_ENV, "WP gabe codes"),
        ] {
            let encoded = account(vault, env, key);
            assert_eq!(
                parse_account(&encoded),
                Some((vault.to_string(), env.to_string(), key.to_string())),
                "round trip failed for {}", encoded
            );
        }
    }

    #[test]
    fn legacy_accounts_read_as_the_default_env() {
        assert_eq!(
            parse_account("mailkite:openai-secret"),
            Some(("mailkite".into(), DEFAULT_ENV.into(), "openai-secret".into()))
        );
        assert_eq!(parse_account("no-colon"), None);
        assert_eq!(parse_account(":empty-vault"), None);
        assert_eq!(parse_account("vault:"), None);
    }

    #[test]
    fn chain_walks_to_the_root_nearest_first() {
        let v = vault_with_chain();
        assert_eq!(v.chain("dev"), vec!["dev", "prod", "default"]);
        assert_eq!(v.chain("prod"), vec!["prod", "default"]);
        assert_eq!(v.chain(DEFAULT_ENV), vec!["default"]);
        // An undeclared environment still resolves to itself rather than panicking.
        assert_eq!(v.chain("ghost"), vec!["ghost"]);
    }

    #[test]
    fn a_cyclic_graph_terminates_instead_of_hanging() {
        let mut v = Vault::default();
        v.envs.insert("a".into(), EnvConfig { extends: Some("b".into()) });
        v.envs.insert("b".into(), EnvConfig { extends: Some("a".into()) });
        assert_eq!(v.chain("a"), vec!["a", "b"]);
    }

    #[test]
    fn cycles_are_detected_before_they_are_written() {
        let v = vault_with_chain();
        assert!(v.would_cycle("prod", "dev"), "prod->dev closes dev->prod");
        assert!(v.would_cycle("dev", "dev"), "self-extension is a cycle");
        assert!(!v.would_cycle("staging", "prod"));
    }

    #[test]
    fn nearest_environment_wins_in_the_resolved_view() {
        let v = vault_with_chain();
        let resolved = v.resolved_keys("dev");
        assert_eq!(
            resolved,
            vec![
                ("api_key".to_string(), "default".to_string()),
                ("db_url".to_string(), "dev".to_string()),
                ("log_level".to_string(), "default".to_string()),
            ]
        );
        // prod overrides db_url but not the rest.
        assert_eq!(
            v.resolved_keys("prod").into_iter().find(|(k, _)| k == "db_url"),
            Some(("db_url".to_string(), "prod".to_string()))
        );
    }

    #[test]
    fn an_implicitly_created_environment_extends_the_root() {
        // `seal set -e dev` must inherit exactly like `seal env add dev`.
        let mut v = Vault::default();
        v.declare_env(DEFAULT_ENV);
        v.declare_env("dev");
        assert_eq!(v.envs[DEFAULT_ENV].extends, None, "the root extends nothing");
        assert_eq!(v.envs["dev"].extends.as_deref(), Some(DEFAULT_ENV));
        assert_eq!(v.chain("dev"), vec!["dev", "default"]);
    }

    #[test]
    fn declaring_an_existing_environment_keeps_its_lineage() {
        let mut v = vault_with_chain();
        v.declare_env("dev");
        assert_eq!(v.envs["dev"].extends.as_deref(), Some("prod"), "dev still extends prod");
    }

    #[test]
    fn v1_indexes_migrate_into_the_default_environment() {
        let index = parse(r#"{"mailkite":["openai-secret"],"empty-vault":[]}"#);
        assert_eq!(index.version, SCHEMA_VERSION);
        let mailkite = index.vault("mailkite").expect("vault carried over");
        assert_eq!(mailkite.own_keys(DEFAULT_ENV), ["openai-secret"]);
        assert!(mailkite.envs.contains_key(DEFAULT_ENV), "default env is implied");
        // A vault declared with no keys must survive the migration.
        assert!(index.vault("empty-vault").is_some());
    }

    #[test]
    fn v2_indexes_round_trip_through_json() {
        let mut index = Index::default();
        let v = index.vault_mut("mailkite");
        v.envs.insert("prod".into(), EnvConfig { extends: Some(DEFAULT_ENV.into()) });
        v.keys.insert("prod".into(), vec!["db_url".into()]);
        let json = serde_json::to_string(&index).unwrap();
        assert_eq!(parse(&json), index);
    }

    #[test]
    fn garbage_parses_as_an_empty_index_rather_than_panicking() {
        assert_eq!(parse("not json at all"), Index::default());
        assert_eq!(parse(""), Index::default());
    }
}
