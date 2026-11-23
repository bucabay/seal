//! Per-item user-presence protection in the macOS Data Protection Keychain.
//!
//! No separate LocalAuthentication preflight: SecItemCopyMatching performs
//! authorization and returns the value in one operation. No values are cached.
#![cfg(target_os = "macos")]

use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords::{
    delete_generic_password_options, generic_password, set_generic_password_options,
    AccessControlOptions, PasswordOptions,
};

const NOT_FOUND: i32 = -25300;
const MISSING_ENTITLEMENT: i32 = -34018;

fn error(e: security_framework::base::Error) -> String {
    match e.code() {
        MISSING_ENTITLEMENT => "Biometric item access requires a signed, provisioned app bundle with keychain-access-groups entitlements. Protected-item access was refused. See docs/BIOMETRICS.md.".into(),
        -128 | -25293 => "Keychain authorization was cancelled or denied".into(),
        code => format!("Protected keychain operation failed (macOS status {})", code),
    }
}

fn options(service: &str, account: &str) -> PasswordOptions {
    let mut options = PasswordOptions::new_generic_password(service, account);
    options.use_protected_keychain();
    options
}

/// Attributes only: listing protected items never authenticates or reads data.
/// Builds without provisioning can use the caller's standard-keychain backend.
pub fn names(service: &str) -> Result<Vec<String>, String> {
    let result = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .ignore_legacy_keychains()
        .load_attributes(true)
        .load_data(false)
        .limit(Limit::All)
        .search();
    match result {
        Ok(items) => Ok(items
            .into_iter()
            .filter_map(|item| item.simplify_dict()?.remove("acct"))
            .collect()),
        Err(e) if e.code() == NOT_FOUND || e.code() == MISSING_ENTITLEMENT => Ok(Vec::new()),
        Err(e) => Err(error(e)),
    }
}

pub fn contains(service: &str, account: &str) -> Result<bool, String> {
    Ok(names(service)?.iter().any(|name| name == account))
}

/// Check signing/entitlements without requesting a password first.
pub fn ensure_available() -> Result<(), String> {
    if available()? {
        Ok(())
    } else {
        Err(error(security_framework::base::Error::from_code(
            MISSING_ENTITLEMENT,
        )))
    }
}

pub fn available() -> Result<bool, String> {
    match ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service("seal-biometric-entitlement-check")
        .ignore_legacy_keychains()
        .load_attributes(true)
        .search()
    {
        Ok(_) => Ok(true),
        Err(e) if e.code() == NOT_FOUND => Ok(true),
        Err(e) if e.code() == MISSING_ENTITLEMENT => Ok(false),
        Err(e) => Err(error(e)),
    }
}

/// Read exactly once. The item's USER_PRESENCE ACL lets macOS offer Touch ID,
/// with the device password fallback. Cancellation is never a legacy fallback.
pub fn get(service: &str, account: &str) -> Result<String, String> {
    let bytes = generic_password(options(service, account)).map_err(error)?;
    String::from_utf8(bytes).map_err(|_| "Keychain value is not UTF-8".into())
}

pub fn set(service: &str, account: &str, value: &str) -> Result<(), String> {
    let mut options = options(service, account);
    if !contains(service, account)? {
        let access = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
            AccessControlOptions::USER_PRESENCE.bits(),
        )
        .map_err(error)?;
        options.set_access_control(access);
    }
    // Existing items update their value without replacing their ACL or reading
    // the old password. New items receive the user-presence policy above.
    set_generic_password_options(value.as_bytes(), options).map_err(error)
}

pub fn delete(service: &str, account: &str) -> Result<(), String> {
    delete_generic_password_options(options(service, account)).map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entitlement_and_cancellation_errors_are_actionable() {
        assert!(error(security_framework::base::Error::from_code(
            MISSING_ENTITLEMENT
        ))
        .contains("signed, provisioned app bundle"));
        assert!(error(security_framework::base::Error::from_code(-128)).contains("cancelled"));
    }

    #[test]
    #[ignore = "requires a signed test binary and interactive Touch ID/password approval"]
    fn protected_item_round_trip() {
        let service = "seal-biometric-selftest";
        let account = format!("test-{}", std::process::id());
        set(service, &account, "test value\n").unwrap();
        assert!(contains(service, &account).unwrap());
        assert_eq!(get(service, &account).unwrap(), "test value\n");
        delete(service, &account).unwrap();
        assert!(!contains(service, &account).unwrap());
    }
}
