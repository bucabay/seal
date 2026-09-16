//! How a reference is read.
//!
//! A reference is `issuer/name` — `stripe/api-key`, `hardroad/db_url`. The
//! first segment is whoever issues the credential: Stripe issues the Stripe
//! key, you issue your own database password.
//!
//! Grouping is **derived from the name**, never stored. There is no such thing
//! as creating a group: name a key `stripe/anything` and the Stripe group
//! exists; remove the last one and it is gone. Nothing to manage, nothing to
//! get out of step with the keys it claims to hold.

/// Where a reference with no issuer is filed.
///
/// A real place to put things rather than a failure state — "ungrouped" or
/// "other" read as leftovers, and a key you deliberately named `scratch` is not
/// a leftover. The reference itself is never rewritten: what you typed is what
/// is stored, and this is only how it files.
pub const DEFAULT_ISSUER: &str = "general";

/// Split a reference into its issuer and the rest.
///
/// Only the first `/` separates: `aws/prod/rds` is issuer `aws`, name
/// `prod/rds`, so a name may itself contain slashes.
pub fn split(reference: &str) -> (&str, &str) {
    match reference.split_once('/') {
        Some((issuer, name)) if !issuer.is_empty() && !name.is_empty() => (issuer, name),
        _ => (DEFAULT_ISSUER, reference),
    }
}

pub fn issuer(reference: &str) -> &str {
    split(reference).0
}

pub fn name(reference: &str) -> &str {
    split(reference).1
}

/// Why a reference was rejected, in words a person can act on.
pub fn problem(reference: &str) -> Option<String> {
    let r = reference.trim();
    if r.is_empty() {
        return Some("a reference is needed, for example stripe/api-key".into());
    }
    if r != reference {
        return Some("a reference cannot begin or end with a space".into());
    }
    if r.chars().any(char::is_whitespace) {
        return Some("a reference cannot contain spaces".into());
    }
    if r.starts_with('/') || r.ends_with('/') {
        return Some("a reference cannot begin or end with `/`".into());
    }
    if r.contains("//") {
        return Some("a reference cannot contain an empty segment".into());
    }
    if r.len() > 512 {
        return Some("that reference is too long".into());
    }
    // Control characters would make a reference unprintable and could confuse
    // anything that logs it.
    if r.chars().any(|c| c.is_control()) {
        return Some("a reference cannot contain control characters".into());
    }
    None
}

pub fn is_valid(reference: &str) -> bool {
    problem(reference).is_none()
}

/// Read a pasted `reference value` line.
///
/// Splitting on the *first* run of whitespace means a value may contain spaces,
/// which passphrases often do. Returns `None` when there is no value part, so a
/// caller can tell "just a reference" from "a reference and a value".
pub fn split_pasted(line: &str) -> (String, Option<String>) {
    let line = line.trim_start();
    match line.find(char::is_whitespace) {
        Some(at) => {
            let (reference, rest) = line.split_at(at);
            let value = rest.trim_start();
            if value.is_empty() {
                (reference.to_string(), None)
            } else {
                (reference.to_string(), Some(value.to_string()))
            }
        }
        None => (line.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_splits_on_its_first_slash() {
        assert_eq!(split("stripe/api-key"), ("stripe", "api-key"));
        assert_eq!(split("hardroad/db_url"), ("hardroad", "db_url"));
        // A name may contain slashes of its own.
        assert_eq!(split("aws/prod/rds"), ("aws", "prod/rds"));
    }

    #[test]
    fn something_without_an_issuer_is_filed_rather_than_refused() {
        assert_eq!(split("looseend"), (DEFAULT_ISSUER, "looseend"));
        assert_eq!(issuer("looseend"), DEFAULT_ISSUER);
        assert_eq!(name("looseend"), "looseend");
    }

    #[test]
    fn a_half_written_reference_does_not_pretend_to_be_grouped() {
        // Mid-typing, `stripe/` has no name yet.
        assert_eq!(split("stripe/"), (DEFAULT_ISSUER, "stripe/"));
        assert_eq!(split("/api-key"), (DEFAULT_ISSUER, "/api-key"));
    }

    #[test]
    fn a_pasted_line_splits_reference_from_value() {
        assert_eq!(
            split_pasted("stripe/api-key sk_xxx"),
            ("stripe/api-key".into(), Some("sk_xxx".into()))
        );
    }

    #[test]
    fn a_value_may_contain_spaces() {
        // Only the first run of whitespace separates; a passphrase survives.
        assert_eq!(
            split_pasted("vault/passphrase correct horse battery staple"),
            (
                "vault/passphrase".into(),
                Some("correct horse battery staple".into())
            )
        );
    }

    #[test]
    fn a_reference_on_its_own_yields_no_value() {
        assert_eq!(
            split_pasted("stripe/api-key"),
            ("stripe/api-key".into(), None)
        );
        assert_eq!(
            split_pasted("stripe/api-key   "),
            ("stripe/api-key".into(), None)
        );
        assert_eq!(
            split_pasted("  stripe/api-key"),
            ("stripe/api-key".into(), None)
        );
    }

    #[test]
    fn tabs_separate_as_well_as_spaces() {
        // Pasting out of a spreadsheet or a password manager.
        assert_eq!(
            split_pasted("stripe/api-key\tsk_xxx"),
            ("stripe/api-key".into(), Some("sk_xxx".into()))
        );
    }

    #[test]
    fn valid_references_are_accepted() {
        for r in [
            "stripe/api-key",
            "hardroad/db_url",
            "aws/prod/rds",
            "looseend",
            "a/b",
        ] {
            assert!(is_valid(r), "`{}` should be valid: {:?}", r, problem(r));
        }
    }

    #[test]
    fn a_bad_reference_says_what_is_wrong() {
        let cases = [
            ("", "is needed"),
            ("has space", "spaces"),
            ("/leading", "begin or end"),
            ("trailing/", "begin or end"),
            ("double//slash", "empty segment"),
            (" padded", "begin or end with a space"),
        ];
        for (input, expected) in cases {
            let said = problem(input).unwrap_or_else(|| panic!("`{}` should be refused", input));
            assert!(
                said.contains(expected),
                "`{}` said `{}`, expected something about `{}`",
                input,
                said,
                expected
            );
        }
    }

    #[test]
    fn control_characters_are_refused() {
        assert!(problem("stripe/api\nkey").is_some());
        assert!(problem("stripe/api\0key").is_some());
    }

    #[test]
    fn a_key_with_no_issuer_keeps_the_name_it_was_given() {
        // Filing it under `general` must not rewrite what the user typed.
        assert_eq!(name("scratch"), "scratch");
        assert_eq!(issuer("scratch"), "general");
        assert!(is_valid("scratch"));
    }

    #[test]
    fn grouping_needs_nothing_stored() {
        // The property that makes groups free: they are a function of the
        // names, so a group cannot disagree with the keys it holds.
        let refs = ["stripe/a", "stripe/b", "github/token", "looseend"];
        let mut groups: Vec<&str> = refs.iter().map(|r| issuer(r)).collect();
        groups.sort();
        groups.dedup();
        assert_eq!(groups, vec![DEFAULT_ISSUER, "github", "stripe"]);
    }
}
