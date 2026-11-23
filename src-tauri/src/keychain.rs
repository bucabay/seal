//! Cross-platform keychain router, preserving individual item authorization.
//!
//! On macOS protected items use the shared native `seal-keychain` library for
//! user-presence authorization (Touch ID or password). The backend below is
//! retained for standard keychain access when biometric provisioning is absent.
//! A protected-item denial never falls back to a legacy password read.
//!
//! Legacy versions created items with `security -A`, permitting prompt-free
//! reads. New standard-keychain saves use an empty trusted-app list instead.
//!
//! On Linux and Windows we keep using the `keyring` crate, which maps to
//! Secret Service and Credential Manager respectively.

const SERVICE: &str = "seal";

#[cfg(target_os = "macos")]
mod backend {
    use std::process::Command;

    pub fn set(service: &str, account: &str, value: &str) -> Result<(), String> {
        let out = Command::new("security")
            .args(["add-generic-password", "-a", account, "-s", service,
                "-w", value, "-T", "", "-U"])
            .output()
            .map_err(|e| format!("failed to run `security`: {}", e))?;
        if out.status.success() { Ok(()) } else {
            Err("Keychain save failed or authorization was denied".into())
        }
    }

    pub fn get(service: &str, account: &str) -> Result<String, String> {
        let out = Command::new("security")
            .args(["find-generic-password", "-a", account, "-s", service, "-w"])
            .output()
            .map_err(|e| format!("failed to run `security`: {}", e))?;

        if out.status.success() {
            let value = String::from_utf8_lossy(&out.stdout).trim_end().to_string();
            if value.is_empty() {
                Err(format!("Not found: {}", account))
            } else {
                Ok(value)
            }
        } else if out.status.code() == Some(44) {
            Err(format!("Not found: {}", account))
        } else {
            Err("Keychain access was cancelled or denied".into())
        }
    }

    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        let out = Command::new("security")
            .args(["delete-generic-password", "-a", account, "-s", service])
            .output()
            .map_err(|e| format!("failed to run `security`: {}", e))?;

        if out.status.success() {
            Ok(())
        } else if out.status.code() == Some(44) {
            Err(format!("Not found: {}", account))
        } else {
            Err("Keychain access was cancelled or denied".into())
        }
    }

    /// Read one quoted attribute value out of a `dump-keychain` line, e.g.
    /// `    "acct"<blob>="hardroad:db_pass"`. Binary values print as `0x…`
    /// followed by the quoted rendering when there is one; unrepresentable
    /// values print as `<NULL>` and are skipped.
    fn attribute(line: &str, name: &str) -> Option<String> {
        let rest = line.trim_start().strip_prefix(&format!("\"{}\"", name))?;
        let value = rest[rest.find('=')? + 1..].trim();
        let open = value.find('"')?;
        let close = value.rfind('"')?;
        (close > open).then(|| value[open + 1..close].to_string())
    }

    /// Every account name stored under `service`.
    ///
    /// `dump-keychain` prints attributes only — reading a *value* needs `-d`,
    /// which we never pass — so this enumerates names without touching any
    /// secret and without prompting.
    pub fn list_accounts(service: &str) -> Option<Vec<String>> {
        let out = Command::new("security").arg("dump-keychain").output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(parse_dump(&String::from_utf8_lossy(&out.stdout), service))
    }

    fn parse_dump(dump: &str, service: &str) -> Vec<String> {
        let mut accounts = Vec::new();
        let (mut acct, mut svce) = (None, None);
        for line in dump.lines() {
            // Each item begins with a `keychain: "…"` header.
            if line.starts_with("keychain:") {
                acct = None;
                svce = None;
                continue;
            }
            if let Some(value) = attribute(line, "acct") {
                acct = Some(value);
            } else if let Some(value) = attribute(line, "svce") {
                svce = Some(value);
            }
            // Attribute order is not guaranteed, so emit as soon as both are in
            // hand rather than assuming acct comes first.
            if svce.as_deref() == Some(service) {
                if let Some(account) = acct.take() {
                    accounts.push(account);
                }
            }
        }
        accounts
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const DUMP: &str = r#"
keychain: "/Users/x/Library/Keychains/login.keychain-db"
class: "genp"
attributes:
    "acct"<blob>="hardroad:db_pass"
    "svce"<blob>="seal"
keychain: "/Users/x/Library/Keychains/login.keychain-db"
class: "genp"
attributes:
    "acct"<blob>="Chrome Safe Storage"
    "svce"<blob>="Chrome"
keychain: "/Users/x/Library/Keychains/login.keychain-db"
class: "genp"
attributes:
    "svce"<blob>="seal"
    "acct"<blob>="Gabe:WP gabe codes"
    "desc"<blob>=<NULL>
"#;

        #[test]
        fn reads_quoted_attribute_values() {
            let line = r#"    "acct"<blob>="hardroad:db_pass""#;
            assert_eq!(attribute(line, "acct").as_deref(), Some("hardroad:db_pass"));
            assert_eq!(attribute(line, "svce"), None);
        }

        #[test]
        fn skips_null_and_unnamed_attributes() {
            assert_eq!(attribute(r#"    "desc"<blob>=<NULL>"#, "desc"), None);
            assert_eq!(attribute("class: \"genp\"", "acct"), None);
        }

        #[test]
        fn collects_only_this_service_in_either_attribute_order() {
            let accounts = parse_dump(DUMP, "seal");
            assert_eq!(accounts, vec!["hardroad:db_pass", "Gabe:WP gabe codes"]);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod backend {
    use keyring::Entry;

    pub fn set(service: &str, account: &str, value: &str) -> Result<(), String> {
        let entry = Entry::new(service, account).map_err(|e| format!("Keyring error: {}", e))?;
        entry
            .set_password(value)
            .map_err(|e| format!("Keyring error: {}", e))
    }

    pub fn get(service: &str, account: &str) -> Result<String, String> {
        let entry = Entry::new(service, account).map_err(|e| format!("Keyring error: {}", e))?;
        entry
            .get_password()
            .map_err(|e| format!("Not found: {} ({})", account, e))
    }

    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        let entry = Entry::new(service, account).map_err(|e| format!("Keyring error: {}", e))?;
        entry
            .delete_credential()
            .map_err(|e| format!("Not found: {} ({})", account, e))
    }

    /// Secret Service and Credential Manager are reachable here only through
    /// `keyring`'s keyed API, which cannot enumerate. `None` means "ask the
    /// index instead".
    pub fn list_accounts(_service: &str) -> Option<Vec<String>> {
        None
    }
}

/// Store a value under `account` (`"{vault}:{key}"`).
pub fn set(account: &str, value: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if seal_keychain::contains(SERVICE, account)? {
            return seal_keychain::set(SERVICE, account, value);
        }
        let legacy = backend::list_accounts(SERVICE).ok_or("Cannot enumerate legacy keychain items")?;
        if !legacy.iter().any(|name| name == account) && seal_keychain::available()? {
            return seal_keychain::set(SERVICE, account, value);
        }
    }
    backend::set(SERVICE, account, value)
}

/// Retrieve the value stored under `account`.
pub fn get(account: &str) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        if seal_keychain::contains(SERVICE, account)? {
            return seal_keychain::get(SERVICE, account);
        }
    }
    backend::get(SERVICE, account)
}

/// Delete the value stored under `account`.
pub fn delete(account: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if seal_keychain::contains(SERVICE, account)? {
        seal_keychain::delete(SERVICE, account)?;
        if backend::list_accounts(SERVICE)
            .ok_or("Cannot enumerate legacy keychain items")?.iter().any(|name| name == account) {
            backend::delete(SERVICE, account)?;
        }
        return Ok(());
    }
    backend::delete(SERVICE, account)
}

/// Every account (`"{vault}:{key}"`) Seal has stored, or `None` on platforms
/// whose keychain cannot be enumerated.
pub fn list_accounts() -> Option<Vec<String>> {
    let accounts = backend::list_accounts(SERVICE);
    #[cfg(target_os = "macos")]
    {
        let mut accounts = accounts?;
        accounts.extend(seal_keychain::names(SERVICE).ok()?);
        accounts.sort();
        accounts.dedup();
        Some(accounts)
    }
    #[cfg(not(target_os = "macos"))]
    { accounts }
}

#[allow(dead_code)] // Used by the GUI, not the CLI.
pub fn biometric_accounts() -> Result<Option<Vec<String>>, String> {
    #[cfg(target_os = "macos")]
    {
        let mut protected = seal_keychain::names(SERVICE)?;
        let legacy = backend::list_accounts(SERVICE).ok_or("Cannot enumerate legacy keychain items")?;
        protected.retain(|name| !legacy.contains(name));
        Ok(Some(protected))
    }
    #[cfg(not(target_os = "macos"))]
    { Ok(None) }
}

pub fn enable_biometrics(account: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        seal_keychain::ensure_available()?;
        if !seal_keychain::contains(SERVICE, account)? {
            let value = backend::get(SERVICE, account)?;
            seal_keychain::set(SERVICE, account, &value)?;
        }
        if backend::list_accounts(SERVICE).ok_or("Cannot enumerate legacy keychain items")?
            .iter().any(|name| name == account) {
            backend::delete(SERVICE, account)?;
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    { let _ = account; Err("Biometric item protection is available on macOS".into()) }
}
