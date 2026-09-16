//! Where values rest, and the type that carries one.
//!
//! [`Secret`] deliberately has no `Display` and a `Debug` that prints nothing
//! useful, so a value cannot reach a log or an error message by accident. The
//! only way out is [`Secret::expose`], which is greppable.

use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    /// Hand over the plaintext. Every call site is a place to look during
    /// review.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret([redacted; {} bytes])", self.0.len())
    }
}

pub trait SecretStore: fmt::Debug {
    fn get(&self, key: &str) -> Result<Secret>;
    fn set(&mut self, key: &str, value: Secret) -> Result<()>;
    fn delete(&mut self, key: &str) -> Result<()>;
    /// Names only. This is the one listing an agent may see.
    fn names(&self) -> Result<Vec<String>>;
}

#[derive(Debug, Default)]
pub struct MemoryStore {
    items: BTreeMap<String, Secret>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with<const N: usize>(pairs: [(&str, &str); N]) -> Self {
        let mut s = MemoryStore::new();
        for (k, v) in pairs {
            s.items.insert(k.to_string(), Secret::new(v));
        }
        s
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Secret> {
        self.items
            .get(key)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("secret `{}`", key)))
    }

    fn set(&mut self, key: &str, value: Secret) -> Result<()> {
        self.items.insert(key.to_string(), value);
        Ok(())
    }

    fn delete(&mut self, key: &str) -> Result<()> {
        self.items
            .remove(key)
            .map(|_| ())
            .ok_or_else(|| Error::NotFound(format!("secret `{}`", key)))
    }

    fn names(&self) -> Result<Vec<String>> {
        Ok(self.items.keys().cloned().collect())
    }
}

/// macOS Keychain, via the `security` CLI.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct KeychainStore {
    service: String,
}

#[cfg(target_os = "macos")]
impl KeychainStore {
    pub fn new(service: impl Into<String>) -> Self {
        KeychainStore { service: service.into() }
    }
}

#[cfg(target_os = "macos")]
impl Default for KeychainStore {
    fn default() -> Self {
        KeychainStore::new("keymaker")
    }
}

#[cfg(target_os = "macos")]
impl SecretStore for KeychainStore {
    fn get(&self, key: &str) -> Result<Secret> {
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-a", key, "-s", &self.service, "-w"])
            .output()
            .map_err(|e| Error::Os(format!("running security: {}", e)))?;
        if !out.status.success() {
            return Err(Error::NotFound(format!("secret `{}`", key)));
        }
        let value = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
        if value.is_empty() {
            return Err(Error::NotFound(format!("secret `{}`", key)));
        }
        Ok(Secret::new(value))
    }

    fn set(&mut self, key: &str, value: Secret) -> Result<()> {
        // `-U` updates in place rather than needing a delete first.
        let out = std::process::Command::new("security")
            .args([
                "add-generic-password",
                "-a",
                key,
                "-s",
                &self.service,
                "-w",
                value.expose(),
                "-U",
            ])
            .output()
            .map_err(|e| Error::Os(format!("running security: {}", e)))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(Error::Store(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    }

    fn delete(&mut self, key: &str) -> Result<()> {
        let out = std::process::Command::new("security")
            .args(["delete-generic-password", "-a", key, "-s", &self.service])
            .output()
            .map_err(|e| Error::Os(format!("running security: {}", e)))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(Error::NotFound(format!("secret `{}`", key)))
        }
    }

    fn names(&self) -> Result<Vec<String>> {
        // `dump-keychain` prints attributes only; printing a value needs `-d`,
        // which is never passed here.
        let out = std::process::Command::new("security")
            .arg("dump-keychain")
            .output()
            .map_err(|e| Error::Os(format!("running security: {}", e)))?;
        if !out.status.success() {
            return Ok(Vec::new());
        }
        Ok(parse_dump(&String::from_utf8_lossy(&out.stdout), &self.service))
    }
}

#[cfg(target_os = "macos")]
fn attribute(line: &str, name: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix(&format!("\"{}\"", name))?;
    let value = rest[rest.find('=')? + 1..].trim();
    let open = value.find('"')?;
    let close = value.rfind('"')?;
    (close > open).then(|| value[open + 1..close].to_string())
}

#[cfg(target_os = "macos")]
fn parse_dump(dump: &str, service: &str) -> Vec<String> {
    let mut names = Vec::new();
    let (mut acct, mut svce) = (None, None);
    for line in dump.lines() {
        if line.starts_with("keychain:") {
            acct = None;
            svce = None;
            continue;
        }
        if let Some(v) = attribute(line, "acct") {
            acct = Some(v);
        } else if let Some(v) = attribute(line, "svce") {
            svce = Some(v);
        }
        if svce.as_deref() == Some(service) {
            if let Some(a) = acct.take() {
                names.push(a);
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_does_not_print_itself() {
        let s = Secret::new("sk_live_do_not_print_me");
        let shown = format!("{:?}", s);
        assert!(!shown.contains("sk_live"), "Debug leaked the value: {}", shown);
        assert!(shown.contains("redacted"));
        assert_eq!(s.expose(), "sk_live_do_not_print_me");
    }

    #[test]
    fn a_secret_inside_a_struct_still_does_not_print() {
        #[derive(Debug)]
        struct Holder {
            #[allow(dead_code)]
            name: String,
            #[allow(dead_code)]
            value: Secret,
        }
        let h = Holder { name: "stripe".into(), value: Secret::new("sk_live_nested") };
        assert!(!format!("{:?}", h).contains("sk_live_nested"));
    }

    #[test]
    fn a_secret_reports_length_without_revealing_content() {
        let s = Secret::new("abcdef");
        assert_eq!(s.len(), 6);
        assert!(!s.is_empty());
        assert!(Secret::new("").is_empty());
    }

    #[test]
    fn memory_store_round_trips() {
        let mut s = MemoryStore::new();
        s.set("stripe/sk", Secret::new("v1")).unwrap();
        assert_eq!(s.get("stripe/sk").unwrap().expose(), "v1");

        s.set("stripe/sk", Secret::new("v2")).unwrap();
        assert_eq!(s.get("stripe/sk").unwrap().expose(), "v2", "set overwrites");

        s.delete("stripe/sk").unwrap();
        assert!(matches!(s.get("stripe/sk"), Err(Error::NotFound(_))));
    }

    #[test]
    fn deleting_something_absent_is_an_error_not_a_silent_success() {
        let mut s = MemoryStore::new();
        assert!(matches!(s.delete("nope"), Err(Error::NotFound(_))));
    }

    #[test]
    fn names_are_sorted_and_contain_no_values() {
        let s = MemoryStore::with([("b/two", "secret2"), ("a/one", "secret1")]);
        let names = s.names().unwrap();
        assert_eq!(names, vec!["a/one", "b/two"]);
        assert!(names.iter().all(|n| !n.contains("secret")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn keychain_dump_parsing_picks_only_this_service() {
        let dump = r#"
keychain: "/Users/x/Library/Keychains/login.keychain-db"
attributes:
    "acct"<blob>="stripe/sk_live"
    "svce"<blob>="keymaker"
keychain: "/Users/x/Library/Keychains/login.keychain-db"
attributes:
    "acct"<blob>="Chrome Safe Storage"
    "svce"<blob>="Chrome"
keychain: "/Users/x/Library/Keychains/login.keychain-db"
attributes:
    "svce"<blob>="keymaker"
    "acct"<blob>="github/token"
"#;
        assert_eq!(parse_dump(dump, "keymaker"), vec!["stripe/sk_live", "github/token"]);
    }
}
