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
    /// Check presence without reading a value when the backend supports it.
    fn contains(&self, key: &str) -> Result<bool> {
        match self.get(key) {
            Ok(_) => Ok(true),
            Err(Error::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
    /// None on platforms without this per-item biometric policy.
    fn biometric_names(&self) -> Result<Option<Vec<String>>> { Ok(None) }
    fn names_are_complete(&self) -> bool { false }
    fn enable_biometrics(&mut self, _key: &str) -> Result<()> {
        Err(Error::Constraint("Biometric item protection is available on macOS".into()))
    }
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
    fn names_are_complete(&self) -> bool { true }
    fn contains(&self, key: &str) -> Result<bool> {
        Ok(self.items.contains_key(key))
    }

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

/// macOS Keychain, with native per-item authorization and legacy upgrade support.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct KeychainStore {
    service: String,
}

#[cfg(target_os = "macos")]
impl KeychainStore {
    pub fn new(service: impl Into<String>) -> Self {
        KeychainStore {
            service: service.into(),
        }
    }

    fn legacy_names(&self) -> Result<Vec<String>> {
        let out = std::process::Command::new("security")
            .arg("dump-keychain")
            .output()
            .map_err(|e| Error::Os(format!("running security: {}", e)))?;
        if !out.status.success() {
            return Err(Error::Store("Cannot enumerate keychain item attributes".into()));
        }
        Ok(parse_dump(&String::from_utf8_lossy(&out.stdout), &self.service))
    }

    fn delete_legacy(&self, key: &str) -> Result<()> {
        let out = std::process::Command::new("security")
            .args(["delete-generic-password", "-a", key, "-s", &self.service])
            .output()
            .map_err(|e| Error::Os(format!("running security: {}", e)))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(Error::Store("Could not remove the legacy keychain item".into()))
        }
    }

    // One standard-keychain read; also used during an explicit biometric upgrade.
    fn legacy_get(&self, key: &str) -> Result<Secret> {
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-a", key, "-s", &self.service, "-w"])
            .output()
            .map_err(|e| Error::Os(format!("running security: {}", e)))?;
        if !out.status.success() {
            return if out.status.code() == Some(44) {
                Err(Error::NotFound(format!("secret `{}`", key)))
            } else {
                Err(Error::Store("Keychain access was cancelled or denied".into()))
            };
        }
        let value = String::from_utf8(out.stdout)
            .map_err(|_| Error::Store("Keychain value is not UTF-8".into()))?;
        Ok(Secret::new(value.strip_suffix('\n').unwrap_or(&value)))
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
    fn biometric_names(&self) -> Result<Option<Vec<String>>> {
        let mut protected = seal_keychain::names(&self.service).map_err(Error::Store)?;
        // A failed migration may have left the old copy. Do not describe an
        // item as protected until that unprotected copy has been removed.
        let legacy = self.legacy_names()?;
        protected.retain(|name| !legacy.contains(name));
        Ok(Some(protected))
    }

    fn names_are_complete(&self) -> bool { true }

    fn enable_biometrics(&mut self, key: &str) -> Result<()> {
        seal_keychain::ensure_available().map_err(Error::Store)?;
        if !seal_keychain::contains(&self.service, key).map_err(Error::Store)? {
            let value = self.legacy_get(key)?;
            seal_keychain::set(&self.service, key, value.expose()).map_err(Error::Store)?;
        }
        // Commit the protected copy before removing the original. Retrying a
        // partial migration only cleans up: it never reads the password again.
        if self.legacy_names()?.iter().any(|name| name == key) {
            self.delete_legacy(key)?;
        }
        Ok(())
    }

    fn contains(&self, key: &str) -> Result<bool> {
        // Item attributes are public metadata. Presence checks must not read
        // passwords: list_refs and health both run during a GUI refresh.
        Ok(self.names()?.iter().any(|name| name == key))
    }

    fn get(&self, key: &str) -> Result<Secret> {
        if seal_keychain::contains(&self.service, key).map_err(Error::Store)? {
            return seal_keychain::get(&self.service, key).map(Secret::new).map_err(Error::Store);
        }
        self.legacy_get(key)
    }

    fn set(&mut self, key: &str, value: Secret) -> Result<()> {
        if seal_keychain::contains(&self.service, key).map_err(Error::Store)? {
            return seal_keychain::set(&self.service, key, value.expose()).map_err(Error::Store);
        }
        if !self.legacy_names()?.iter().any(|name| name == key)
            && seal_keychain::available().map_err(Error::Store)? {
            return seal_keychain::set(&self.service, key, value.expose()).map_err(Error::Store);
        }
        let out = std::process::Command::new("security")
            .args(["add-generic-password", "-a", key, "-s", &self.service,
                "-w", value.expose(), "-T", "", "-U"])
            .output()
            .map_err(|e| Error::Os(format!("running security: {}", e)))?;
        if out.status.success() { Ok(()) } else {
            Err(Error::Store("Keychain save failed or authorization was denied".into()))
        }
    }

    fn delete(&mut self, key: &str) -> Result<()> {
        if seal_keychain::contains(&self.service, key).map_err(Error::Store)? {
            seal_keychain::delete(&self.service, key).map_err(Error::Store)?;
            if self.legacy_names()?.iter().any(|name| name == key) {
                self.delete_legacy(key)?;
            }
            return Ok(());
        }
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
        let mut names = self.legacy_names()?;
        names.extend(seal_keychain::names(&self.service).map_err(Error::Store)?);
        names.sort();
        names.dedup();
        Ok(names)
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

/// Linux Secret Service and Windows Credential Manager, through `keyring`.
///
/// macOS is handled separately above, because `keyring`'s macOS backend creates
/// items with an ACL that re-prompts across binaries.
#[cfg(any(all(unix, not(target_os = "macos")), windows))]
#[derive(Debug, Clone)]
pub struct KeyringStore {
    service: String,
}

#[cfg(any(all(unix, not(target_os = "macos")), windows))]
impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        KeyringStore {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, key).map_err(|e| Error::Store(format!("keyring: {}", e)))
    }
}

#[cfg(any(all(unix, not(target_os = "macos")), windows))]
impl Default for KeyringStore {
    fn default() -> Self {
        KeyringStore::new("keymaker")
    }
}

#[cfg(any(all(unix, not(target_os = "macos")), windows))]
impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Secret> {
        self.entry(key)?
            .get_password()
            .map(Secret::new)
            .map_err(|_| Error::NotFound(format!("secret `{}`", key)))
    }

    fn set(&mut self, key: &str, value: Secret) -> Result<()> {
        self.entry(key)?
            .set_password(value.expose())
            .map_err(|e| Error::Store(format!("keyring: {}", e)))
    }

    fn delete(&mut self, key: &str) -> Result<()> {
        self.entry(key)?
            .delete_credential()
            .map_err(|_| Error::NotFound(format!("secret `{}`", key)))
    }

    /// Neither backend can be enumerated through the keyed API used here, so
    /// listing falls back to the names recorded in the manifest.
    ///
    /// Returning an empty list rather than an error keeps `doctor` and the
    /// broker's listings working: they report what a manifest asks for and
    /// check each reference individually, which does not need enumeration.
    fn names(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// The right store for this platform.
pub fn platform_store() -> Box<dyn SecretStore> {
    #[cfg(target_os = "macos")]
    {
        Box::new(KeychainStore::default())
    }
    #[cfg(any(all(unix, not(target_os = "macos")), windows))]
    {
        Box::new(KeyringStore::default())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Box::new(MemoryStore::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_does_not_print_itself() {
        let s = Secret::new("sk_live_do_not_print_me");
        let shown = format!("{:?}", s);
        assert!(
            !shown.contains("sk_live"),
            "Debug leaked the value: {}",
            shown
        );
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
        let h = Holder {
            name: "stripe".into(),
            value: Secret::new("sk_live_nested"),
        };
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

    #[test]
    #[cfg_attr(target_os = "macos", ignore = "requires interactive OS-keychain approval")]
    fn the_platform_store_round_trips_a_value() {
        // Exercises whichever backend this platform actually uses, so the
        // cfg-gated code is not merely compiled but run.
        let mut store = platform_store();
        let key = format!("keymaker-selftest/{}", std::process::id());
        let value = "round-trip-value";

        store.set(&key, Secret::new(value)).expect("set");
        assert_eq!(store.get(&key).expect("get").expose(), value);
        store.delete(&key).expect("delete");
        assert!(store.get(&key).is_err(), "a deleted secret must be gone");
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
        assert_eq!(
            parse_dump(dump, "keymaker"),
            vec!["stripe/sk_live", "github/token"]
        );
    }
}
