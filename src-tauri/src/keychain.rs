//! Cross-platform keychain backend.
//!
//! On macOS we shell out to the `security` CLI and create items with the `-A`
//! flag (allow any application to access without warning). The `keyring`
//! crate's macOS backend creates items with a restrictive ACL, which makes
//! macOS re-prompt (and sometimes fail with "not allowed") whenever the GUI
//! and CLI — or dev vs. release builds — are different binaries.
//!
//! On Linux and Windows we keep using the `keyring` crate, which maps to
//! Secret Service and Credential Manager respectively.

const SERVICE: &str = "seal";

#[cfg(target_os = "macos")]
mod backend {
    use std::process::Command;

    pub fn set(service: &str, account: &str, value: &str) -> Result<(), String> {
        // Delete any existing item first so `-A` is freshly applied.
        let _ = Command::new("security")
            .args(["delete-generic-password", "-a", account, "-s", service])
            .output();

        let out = Command::new("security")
            .args([
                "add-generic-password",
                "-a",
                account,
                "-s",
                service,
                "-w",
                value,
                "-A",
            ])
            .output()
            .map_err(|e| format!("failed to run `security`: {}", e))?;

        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
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
        } else {
            Err(format!("Not found: {}", account))
        }
    }

    pub fn delete(service: &str, account: &str) -> Result<(), String> {
        let out = Command::new("security")
            .args(["delete-generic-password", "-a", account, "-s", service])
            .output()
            .map_err(|e| format!("failed to run `security`: {}", e))?;

        if out.status.success() {
            Ok(())
        } else {
            Err(format!("Not found: {}", account))
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
}

/// Store a value under `account` (`"{vault}:{key}"`).
pub fn set(account: &str, value: &str) -> Result<(), String> {
    backend::set(SERVICE, account, value)
}

/// Retrieve the value stored under `account`.
pub fn get(account: &str) -> Result<String, String> {
    backend::get(SERVICE, account)
}

/// Delete the value stored under `account`.
pub fn delete(account: &str) -> Result<(), String> {
    backend::delete(SERVICE, account)
}
