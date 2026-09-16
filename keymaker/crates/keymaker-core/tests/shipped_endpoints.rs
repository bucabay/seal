//! Every shipped endpoint definition must be valid and safe.
//!
//! A definition is a security control, not documentation: it decides what a
//! credential may be used for. A broken or careless one is a hole, so the whole
//! shipped set is checked here rather than trusted.

use keymaker_core::provider::{Catalog, Endpoint, Injection, RequestDraft};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn endpoints_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/keymaker-core.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../endpoints")
        .canonicalize()
        .expect("endpoints directory")
}

fn all() -> Vec<(String, Endpoint)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(endpoints_dir()).expect("read endpoints") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read definition");
        let catalog = Catalog::from_toml(&src)
            .unwrap_or_else(|e| panic!("{} does not parse: {}", path.display(), e));
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        for ep in catalog.endpoints {
            out.push((file.clone(), ep));
        }
    }
    assert!(
        !out.is_empty(),
        "no definitions found in {}",
        endpoints_dir().display()
    );
    out
}

#[test]
fn every_definition_parses() {
    let eps = all();
    assert!(
        eps.len() >= 10,
        "expected the shipped set, found {}",
        eps.len()
    );
}

#[test]
fn every_name_is_unique_across_files() {
    let mut seen = BTreeMap::new();
    for (file, ep) in all() {
        if let Some(other) = seen.insert(ep.name.clone(), file.clone()) {
            panic!("`{}` is defined in both {} and {}", ep.name, other, file);
        }
    }
}

#[test]
fn every_definition_pins_a_host_and_a_concrete_method() {
    for (file, ep) in all() {
        assert!(
            !ep.host.is_empty() && !ep.host.contains('*') && !ep.host.contains('{'),
            "{}: `{}` must pin an exact host, found `{}`",
            file,
            ep.name,
            ep.host
        );
        assert!(
            ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&ep.method.as_str()),
            "{}: `{}` has method `{}`",
            file,
            ep.name,
            ep.method
        );
        assert!(
            ep.path.starts_with('/'),
            "{}: `{}` path must be absolute",
            file,
            ep.name
        );
    }
}

#[test]
fn no_definition_carries_a_secret_value() {
    // A definition names where the credential lives; it never contains one.
    for entry in std::fs::read_dir(endpoints_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        for marker in ["sk_live_", "sk-ant-", "ghp_", "AKIA", "-----BEGIN"] {
            assert!(
                !src.contains(marker),
                "{} looks like it contains a real credential (`{}`)",
                path.display(),
                marker
            );
        }
    }
}

#[test]
fn the_injected_header_is_never_also_caller_supplied() {
    for (file, ep) in all() {
        if let Injection::Header { name, .. } = &ep.inject {
            let lower = name.to_ascii_lowercase();
            assert!(
                !ep.allow_headers
                    .iter()
                    .any(|h| h.to_ascii_lowercase() == lower),
                "{}: `{}` allows the caller to supply `{}`, which carries the credential",
                file,
                ep.name,
                name
            );
            assert!(
                !ep.headers.keys().any(|h| h.to_ascii_lowercase() == lower),
                "{}: `{}` sets `{}` as a fixed header and also injects into it",
                file,
                ep.name,
                name
            );
        }
    }
}

#[test]
fn money_moving_endpoints_require_a_human() {
    // A judgement encoded as a test: an agent must not move money unattended.
    let eps = all();
    let payout = eps
        .iter()
        .find(|(_, e)| e.name == "stripe.payout_create")
        .expect("stripe.payout_create should be defined");
    assert!(
        payout.1.policy.always_step_up,
        "creating a payout must always stop for a person"
    );

    let refund = eps
        .iter()
        .find(|(_, e)| e.name == "stripe.refund")
        .expect("stripe.refund should be defined");
    assert!(
        refund.1.policy.step_up.is_some(),
        "a large refund must stop for a person"
    );
}

#[test]
fn a_representative_request_builds_against_each_service() {
    let eps = all();
    let find = |name: &str| -> Endpoint {
        eps.iter()
            .find(|(_, e)| e.name == name)
            .unwrap_or_else(|| panic!("`{}` should be defined", name))
            .1
            .clone()
    };

    // Anthropic: fixed version header, credential in x-api-key.
    let ep = find("anthropic.messages");
    let draft = RequestDraft {
        endpoint: "anthropic.messages".into(),
        body: [
            ("model".to_string(), serde_json::json!("claude-opus-5")),
            ("max_tokens".to_string(), serde_json::json!(1024)),
            (
                "messages".to_string(),
                serde_json::json!([{"role": "user", "content": "hello"}]),
            ),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let req = ep.build(&draft, "test-key").expect("anthropic request");
    assert_eq!(req.url, "https://api.anthropic.com/v1/messages");
    assert_eq!(req.headers.get("x-api-key").unwrap(), "test-key");
    assert_eq!(req.headers.get("anthropic-version").unwrap(), "2023-06-01");

    // GitHub: path parameters, bearer credential.
    let ep = find("github.issue_create");
    let draft = RequestDraft {
        endpoint: "github.issue_create".into(),
        path_params: [
            ("owner".to_string(), "bucabay".to_string()),
            ("repo".to_string(), "seal".to_string()),
        ]
        .into_iter()
        .collect(),
        body: [("title".to_string(), serde_json::json!("hello"))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let req = ep.build(&draft, "ghtoken").expect("github request");
    assert_eq!(req.url, "https://api.github.com/repos/bucabay/seal/issues");
    assert_eq!(req.headers.get("Authorization").unwrap(), "Bearer ghtoken");

    // Stripe: form encoding.
    let ep = find("stripe.refund");
    let draft = RequestDraft {
        endpoint: "stripe.refund".into(),
        body: [
            ("charge".to_string(), serde_json::json!("ch_123")),
            ("amount".to_string(), serde_json::json!(500)),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let req = ep.build(&draft, "sk_test").expect("stripe request");
    assert_eq!(req.content_type, "application/x-www-form-urlencoded");
    assert_eq!(req.body.unwrap(), "amount=500&charge=ch_123");
}

#[test]
fn a_large_refund_stops_before_the_credential_is_touched() {
    let ep = all()
        .into_iter()
        .find(|(_, e)| e.name == "stripe.refund")
        .unwrap()
        .1;
    let draft = RequestDraft {
        endpoint: "stripe.refund".into(),
        body: [
            ("charge".to_string(), serde_json::json!("ch_123")),
            ("amount".to_string(), serde_json::json!(500_000)),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    assert!(
        matches!(
            ep.build(&draft, "sk_test"),
            Err(keymaker_core::Error::StepUpRequired(_))
        ),
        "a £5,000 refund should require approval"
    );
}

#[test]
fn a_caller_cannot_redirect_a_definition_to_another_endpoint() {
    let ep = all()
        .into_iter()
        .find(|(_, e)| e.name == "github.issue_create")
        .unwrap()
        .1;
    // A path parameter that tries to walk somewhere else.
    let draft = RequestDraft {
        endpoint: "github.issue_create".into(),
        path_params: [
            ("owner".to_string(), "bucabay".to_string()),
            ("repo".to_string(), "../../user/repos".to_string()),
        ]
        .into_iter()
        .collect(),
        body: [("title".to_string(), serde_json::json!("x"))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    assert!(ep.check(&draft).is_err(), "path traversal must be refused");
}
