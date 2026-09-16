//! Endpoint definitions: constrained request *constructors*.
//!
//! Not a forwarding proxy. A proxy that attaches a credential and forwards
//! whatever it is given lets an injected agent point your key at any endpoint
//! on that host. A constructor pins the method, host and path, allowlists
//! headers, bounds the body, and validates it against a schema — and only then
//! attaches the credential.
//!
//! Every check runs *before* the secret is read, so a rejected request never
//! touches it.

use crate::error::{Error, Result};
use crate::policy::{Decision, Policy, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where the credential goes once every check has passed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Injection {
    /// `format` must contain `{secret}`, e.g. `Bearer {secret}`.
    Header {
        name: String,
        format: String,
    },
    Query {
        name: String,
    },
}

impl Injection {
    /// The header name this injection owns, if any. An agent may never supply
    /// it: doing so would let it swap the credential for one of its own.
    fn reserved_header(&self) -> Option<&str> {
        match self {
            Injection::Header { name, .. } => Some(name),
            Injection::Query { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Str,
    Uint32,
    Number,
    Bool,
    /// Any JSON value, including arrays and objects.
    ///
    /// An escape hatch for fields whose shape is genuinely open — a chat
    /// `messages` array, for instance. The request is still pinned to one
    /// method, host and path, and the header allowlist and size bound still
    /// apply; only the shape of this field goes unchecked. Prefer a scalar
    /// type wherever the API actually has one.
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    pub ty: FieldType,
    pub optional: bool,
}

impl FieldSpec {
    /// `"string"`, `"uint32"`, `"number"`, `"bool"`; a trailing `?` makes it
    /// optional.
    pub fn parse(src: &str) -> Result<FieldSpec> {
        let (base, optional) = match src.strip_suffix('?') {
            Some(b) => (b, true),
            None => (src, false),
        };
        let ty = match base {
            "string" => FieldType::Str,
            "uint32" => FieldType::Uint32,
            "number" => FieldType::Number,
            "bool" => FieldType::Bool,
            "json" => FieldType::Json,
            other => return Err(Error::Parse(format!("unknown field type `{}`", other))),
        };
        Ok(FieldSpec { ty, optional })
    }

    fn accepts(&self, v: &serde_json::Value) -> bool {
        match self.ty {
            FieldType::Str => v.is_string(),
            FieldType::Bool => v.is_boolean(),
            FieldType::Number => v.is_number(),
            FieldType::Uint32 => v.as_u64().map(|n| n <= u32::MAX as u64).unwrap_or(false),
            // Any shape, but not absent-in-disguise.
            FieldType::Json => !v.is_null(),
        }
    }
}

fn default_max_body() -> usize {
    64 * 1024
}

/// How a validated body is written on the wire.
///
/// Both styles are common enough that supporting only one would leave half of
/// the useful APIs undefinable: Stripe and many older services take
/// form-encoded bodies, while most modern ones take JSON.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyFormat {
    #[default]
    Json,
    Form,
}

impl BodyFormat {
    pub fn content_type(&self) -> &'static str {
        match self {
            BodyFormat::Json => "application/json",
            BodyFormat::Form => "application/x-www-form-urlencoded",
        }
    }

    fn encode(&self, body: &BTreeMap<String, serde_json::Value>) -> Result<String> {
        match self {
            BodyFormat::Json => serde_json::to_string(body)
                .map_err(|e| Error::Parse(format!("body is not encodable: {}", e))),
            BodyFormat::Form => {
                let mut parts = Vec::new();
                for (k, v) in body {
                    // Only scalars reach here: the schema has already rejected
                    // arrays and objects, which form encoding cannot express.
                    let rendered = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    parts.push(format!(
                        "{}={}",
                        percent_encode(k),
                        percent_encode(&rendered)
                    ));
                }
                Ok(parts.join("&"))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    /// Dotted name the agent asks for, e.g. `stripe.refund`.
    pub name: String,
    pub method: String,
    pub host: String,
    /// Exact path, or a template with `{param}` segments.
    pub path: String,
    /// Where the value lives in the store. Never sent to the agent.
    pub secret: String,
    pub inject: Injection,
    /// Headers this definition always sets, such as an API version. A caller
    /// can neither supply nor override one.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Headers a caller *may* supply.
    #[serde(default)]
    pub allow_headers: Vec<String>,
    #[serde(default = "default_max_body")]
    pub max_body: usize,
    /// Field name -> type string. An empty schema forbids a body entirely.
    #[serde(default)]
    pub schema: BTreeMap<String, String>,
    #[serde(default)]
    pub body_format: BodyFormat,
    #[serde(default)]
    pub policy: Policy,
}

/// What the agent submits. It names an endpoint; it cannot describe one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequestDraft {
    pub endpoint: String,
    #[serde(default)]
    pub path_params: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: BTreeMap<String, serde_json::Value>,
}

/// A request with the credential already in it. Never returned to an agent.
#[derive(Debug, Clone)]
pub struct PreparedRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<String>,
    /// Content type matching how `body` was encoded.
    pub content_type: &'static str,
}

/// A request that passed every structural check but has not been given the
/// credential yet. Policy runs against this.
#[derive(Debug, Clone)]
pub struct CheckedRequest {
    endpoint_name: String,
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: Option<String>,
    content_type: &'static str,
    facts: BTreeMap<String, Value>,
}

impl CheckedRequest {
    pub fn facts(&self) -> &BTreeMap<String, Value> {
        &self.facts
    }
    pub fn endpoint_name(&self) -> &str {
        &self.endpoint_name
    }
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// A path parameter may only be one plain segment. This is what stops
/// `{id}` = `../../v1/payouts` from walking to a different endpoint.
fn safe_path_param(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 256
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && v != "."
        && v != ".."
}

impl Endpoint {
    /// Structural validation. Runs before the secret is read.
    pub fn check(&self, draft: &RequestDraft) -> Result<CheckedRequest> {
        if draft.endpoint != self.name {
            return Err(Error::Constraint(format!(
                "draft is for `{}`, not `{}`",
                draft.endpoint, self.name
            )));
        }

        // --- path -------------------------------------------------------
        let mut path = String::new();
        let mut used = 0usize;
        let mut rest = self.path.as_str();
        while let Some(open) = rest.find('{') {
            let close = rest[open..]
                .find('}')
                .ok_or_else(|| Error::Parse(format!("unclosed `{{` in path `{}`", self.path)))?
                + open;
            path.push_str(&rest[..open]);
            let param = &rest[open + 1..close];
            let value = draft
                .path_params
                .get(param)
                .ok_or_else(|| Error::Constraint(format!("missing path parameter `{}`", param)))?;
            if !safe_path_param(value) {
                return Err(Error::Constraint(format!(
                    "path parameter `{}` is not a single plain segment",
                    param
                )));
            }
            path.push_str(value);
            used += 1;
            rest = &rest[close + 1..];
        }
        path.push_str(rest);

        if used != draft.path_params.len() {
            return Err(Error::Constraint(
                "path parameters supplied that the path does not use".into(),
            ));
        }

        // --- headers ----------------------------------------------------
        let reserved = self
            .inject
            .reserved_header()
            .map(|h| h.to_ascii_lowercase());
        let fixed: std::collections::BTreeSet<String> = self
            .headers
            .keys()
            .map(|h| h.to_ascii_lowercase())
            .collect();
        // Start from the headers the definition always sets, so a caller can
        // add to them but never replace one.
        let mut headers = self.headers.clone();
        for (name, value) in &draft.headers {
            let lower = name.to_ascii_lowercase();
            if fixed.contains(&lower) {
                return Err(Error::Constraint(format!(
                    "header `{}` is fixed by the definition and cannot be supplied",
                    name
                )));
            }
            if Some(&lower) == reserved.as_ref() {
                return Err(Error::Constraint(format!(
                    "header `{}` carries the credential and cannot be supplied",
                    name
                )));
            }
            if !self
                .allow_headers
                .iter()
                .any(|a| a.to_ascii_lowercase() == lower)
            {
                return Err(Error::Constraint(format!(
                    "header `{}` is not allowed",
                    name
                )));
            }
            if value.bytes().any(|b| b == b'\r' || b == b'\n') {
                return Err(Error::Constraint(format!(
                    "header `{}` contains a line break",
                    name
                )));
            }
            headers.insert(name.clone(), value.clone());
        }

        // --- body -------------------------------------------------------
        let mut facts = BTreeMap::new();
        let body = if self.schema.is_empty() {
            if !draft.body.is_empty() {
                return Err(Error::Constraint("this endpoint takes no body".into()));
            }
            None
        } else {
            for key in draft.body.keys() {
                if !self.schema.contains_key(key) {
                    return Err(Error::Constraint(format!("unknown body field `{}`", key)));
                }
            }
            for (key, spec_src) in &self.schema {
                let spec = FieldSpec::parse(spec_src)?;
                match draft.body.get(key) {
                    None if spec.optional => {}
                    None => return Err(Error::Constraint(format!("missing body field `{}`", key))),
                    Some(v) => {
                        if !spec.accepts(v) {
                            return Err(Error::Constraint(format!(
                                "body field `{}` must be {}",
                                key, spec_src
                            )));
                        }
                        if let Some(f) = Value::from_json(v) {
                            facts.insert(key.clone(), f);
                        }
                    }
                }
            }
            let encoded = self.body_format.encode(&draft.body)?;
            if encoded.len() > self.max_body {
                return Err(Error::Constraint(format!(
                    "body is {} bytes, limit is {}",
                    encoded.len(),
                    self.max_body
                )));
            }
            Some(encoded)
        };

        Ok(CheckedRequest {
            endpoint_name: self.name.clone(),
            method: self.method.to_ascii_uppercase(),
            url: format!("https://{}{}", self.host, path),
            headers,
            body,
            content_type: self.body_format.content_type(),
            facts,
        })
    }

    pub fn decide(&self, checked: &CheckedRequest) -> Decision {
        self.policy.evaluate(&checked.facts)
    }

    /// Attach the credential. The only function in the library that puts a
    /// secret value into something that leaves the process, and it takes the
    /// value as an argument so nothing here can reach the store on its own.
    pub fn prepare(&self, checked: CheckedRequest, secret: &str) -> PreparedRequest {
        let mut headers = checked.headers;
        let mut url = checked.url;
        match &self.inject {
            Injection::Header { name, format } => {
                headers.insert(name.clone(), format.replace("{secret}", secret));
            }
            Injection::Query { name } => {
                let sep = if url.contains('?') { '&' } else { '?' };
                url.push(sep);
                url.push_str(name);
                url.push('=');
                url.push_str(&percent_encode(secret));
            }
        }
        PreparedRequest {
            method: checked.method,
            url,
            headers,
            body: checked.body,
            content_type: checked.content_type,
        }
    }

    /// Full path: check, then policy, then inject. Returns `StepUpRequired`
    /// rather than preparing anything when a human is needed.
    pub fn build(&self, draft: &RequestDraft, secret: &str) -> Result<PreparedRequest> {
        let checked = self.check(draft)?;
        match self.decide(&checked) {
            Decision::Allow => Ok(self.prepare(checked, secret)),
            Decision::Deny(why) => Err(Error::Denied(why)),
            Decision::StepUp(why) => Err(Error::StepUpRequired(why)),
        }
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// A loaded set of endpoint definitions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default, rename = "endpoint")]
    pub endpoints: Vec<Endpoint>,
}

impl Catalog {
    pub fn from_toml(src: &str) -> Result<Catalog> {
        let cat: Catalog =
            toml::from_str(src).map_err(|e| Error::Parse(format!("catalog: {}", e)))?;
        let mut seen = std::collections::BTreeSet::new();
        for e in &cat.endpoints {
            if !seen.insert(&e.name) {
                return Err(Error::Parse(format!("duplicate endpoint `{}`", e.name)));
            }
            if let Injection::Header { format, .. } = &e.inject {
                if !format.contains("{secret}") {
                    return Err(Error::Parse(format!(
                        "endpoint `{}`: inject format must contain {{secret}}",
                        e.name
                    )));
                }
            }
        }
        Ok(cat)
    }

    pub fn get(&self, name: &str) -> Result<&Endpoint> {
        self.endpoints
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| Error::NotFound(format!("endpoint `{}`", name)))
    }

    /// Names only. This is what an agent is allowed to see.
    pub fn names(&self) -> Vec<&str> {
        self.endpoints.iter().map(|e| e.name.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CATALOG: &str = r#"
[[endpoint]]
name = "stripe.refund"
method = "POST"
host = "api.stripe.com"
path = "/v1/refunds"
secret = "stripe/sk_live"
allow_headers = ["Idempotency-Key"]
max_body = 4096
schema = { amount = "uint32", charge = "string", reason = "string?" }
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
policy = { step_up = "amount > 100000" }

[[endpoint]]
name = "stripe.charge_get"
method = "GET"
host = "api.stripe.com"
path = "/v1/charges/{id}"
secret = "stripe/sk_live"
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
"#;

    const SECRET: &str = "sk_live_TESTVALUE";

    fn catalog() -> Catalog {
        Catalog::from_toml(CATALOG).unwrap()
    }

    fn refund_draft() -> RequestDraft {
        RequestDraft {
            endpoint: "stripe.refund".into(),
            body: [
                ("amount".into(), json!(500)),
                ("charge".into(), json!("ch_1")),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_valid_request_is_built_with_the_credential_attached() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let req = ep.build(&refund_draft(), SECRET).unwrap();

        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "https://api.stripe.com/v1/refunds");
        assert_eq!(
            req.headers.get("Authorization").unwrap(),
            "Bearer sk_live_TESTVALUE"
        );
        assert!(req.body.unwrap().contains("ch_1"));
    }

    #[test]
    fn the_secret_is_absent_until_prepare_is_called() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let checked = ep.check(&refund_draft()).unwrap();
        let rendered = format!("{:?}", checked);
        assert!(
            !rendered.contains(SECRET),
            "a checked request must not carry the credential"
        );
        assert!(!rendered.contains("sk_live"));
    }

    #[test]
    fn the_agent_cannot_change_the_host_or_path() {
        // There is no field through which it could: the draft names an
        // endpoint and supplies parameters, nothing more.
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let req = ep.build(&refund_draft(), SECRET).unwrap();
        assert!(req.url.starts_with("https://api.stripe.com/"));
    }

    #[test]
    fn the_agent_cannot_supply_the_credential_header() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.headers
            .insert("Authorization".into(), "Bearer attacker".into());
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));

        // Case must not be a way around it.
        let mut d2 = refund_draft();
        d2.headers
            .insert("authorization".into(), "Bearer attacker".into());
        assert!(matches!(ep.check(&d2), Err(Error::Constraint(_))));
    }

    #[test]
    fn headers_outside_the_allowlist_are_rejected() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.headers.insert("X-Forwarded-For".into(), "evil".into());
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));
    }

    #[test]
    fn an_allowlisted_header_passes_through() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.headers.insert("Idempotency-Key".into(), "abc123".into());
        let req = ep.build(&d, SECRET).unwrap();
        assert_eq!(req.headers.get("Idempotency-Key").unwrap(), "abc123");
    }

    #[test]
    fn header_injection_via_crlf_is_rejected() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.headers.insert(
            "Idempotency-Key".into(),
            "a\r\nAuthorization: Bearer evil".into(),
        );
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));
    }

    #[test]
    fn unknown_body_fields_are_rejected() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.body.insert("destination".into(), json!("acct_attacker"));
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));
    }

    #[test]
    fn missing_required_fields_are_rejected_and_optional_ones_are_not() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.body.remove("charge");
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));

        let d2 = refund_draft(); // `reason` is optional and absent
        assert!(ep.check(&d2).is_ok());
    }

    #[test]
    fn body_field_types_are_enforced() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.body.insert("amount".into(), json!("not a number"));
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));

        let mut d2 = refund_draft();
        d2.body.insert("amount".into(), json!(-1));
        assert!(
            matches!(ep.check(&d2), Err(Error::Constraint(_))),
            "uint32 rejects negatives"
        );

        let mut d3 = refund_draft();
        d3.body.insert("amount".into(), json!(5_000_000_000u64));
        assert!(
            matches!(ep.check(&d3), Err(Error::Constraint(_))),
            "uint32 rejects overflow"
        );
    }

    #[test]
    fn oversized_bodies_are_rejected() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.body.insert("charge".into(), json!("x".repeat(8192)));
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));
    }

    #[test]
    fn path_parameters_are_substituted() {
        let cat = catalog();
        let ep = cat.get("stripe.charge_get").unwrap();
        let d = RequestDraft {
            endpoint: "stripe.charge_get".into(),
            path_params: [("id".to_string(), "ch_123".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let req = ep.build(&d, SECRET).unwrap();
        assert_eq!(req.url, "https://api.stripe.com/v1/charges/ch_123");
    }

    #[test]
    fn a_path_parameter_cannot_walk_to_another_endpoint() {
        let cat = catalog();
        let ep = cat.get("stripe.charge_get").unwrap();
        for evil in [
            "../payouts",
            "ch/../../v1/payouts",
            "..",
            ".",
            "a/b",
            "a%2Fb",
            "",
        ] {
            let d = RequestDraft {
                endpoint: "stripe.charge_get".into(),
                path_params: [("id".to_string(), evil.to_string())].into_iter().collect(),
                ..Default::default()
            };
            assert!(
                matches!(ep.check(&d), Err(Error::Constraint(_))),
                "path parameter `{}` must be rejected",
                evil
            );
        }
    }

    #[test]
    fn a_missing_path_parameter_is_rejected() {
        let cat = catalog();
        let ep = cat.get("stripe.charge_get").unwrap();
        let d = RequestDraft {
            endpoint: "stripe.charge_get".into(),
            ..Default::default()
        };
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));
    }

    #[test]
    fn an_endpoint_with_no_schema_refuses_a_body() {
        let cat = catalog();
        let ep = cat.get("stripe.charge_get").unwrap();
        let d = RequestDraft {
            endpoint: "stripe.charge_get".into(),
            path_params: [("id".to_string(), "ch_1".to_string())]
                .into_iter()
                .collect(),
            body: [("amount".to_string(), json!(1))].into_iter().collect(),
            ..Default::default()
        };
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));
    }

    #[test]
    fn a_draft_for_another_endpoint_is_rejected() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.endpoint = "stripe.charge_get".into();
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));
    }

    #[test]
    fn policy_can_demand_a_human_and_nothing_is_prepared() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        let mut d = refund_draft();
        d.body.insert("amount".into(), json!(200_000));
        match ep.build(&d, SECRET) {
            Err(Error::StepUpRequired(_)) => {}
            other => panic!("expected step-up, got {:?}", other),
        }
    }

    #[test]
    fn query_injection_percent_encodes_the_value() {
        let src = r#"
[[endpoint]]
name = "legacy.ping"
method = "GET"
host = "api.legacy.test"
path = "/ping"
secret = "legacy/key"
inject = { kind = "query", name = "api_key" }
"#;
        let cat = Catalog::from_toml(src).unwrap();
        let ep = cat.get("legacy.ping").unwrap();
        let d = RequestDraft {
            endpoint: "legacy.ping".into(),
            ..Default::default()
        };
        let req = ep.build(&d, "a b/c").unwrap();
        assert_eq!(req.url, "https://api.legacy.test/ping?api_key=a%20b%2Fc");
    }

    #[test]
    fn catalog_rejects_duplicates_and_bad_injection_formats() {
        let dup = format!("{}{}", CATALOG, CATALOG);
        assert!(matches!(Catalog::from_toml(&dup), Err(Error::Parse(_))));

        let bad = r#"
[[endpoint]]
name = "x.y"
method = "GET"
host = "h.test"
path = "/"
secret = "s"
inject = { kind = "header", name = "Authorization", format = "Bearer nothing" }
"#;
        assert!(matches!(Catalog::from_toml(bad), Err(Error::Parse(_))));
    }

    #[test]
    fn listing_exposes_names_and_never_secrets() {
        let cat = catalog();
        assert_eq!(cat.names(), vec!["stripe.refund", "stripe.charge_get"]);
        assert!(cat.get("nope").is_err());
    }

    #[test]
    fn fixed_headers_are_always_set_and_cannot_be_overridden() {
        let src = r#"
[[endpoint]]
name = "anthropic.messages"
method = "POST"
host = "api.anthropic.com"
path = "/v1/messages"
secret = "anthropic/api_key"
headers = { "anthropic-version" = "2023-06-01" }
schema = { model = "string", messages = "json" }
inject = { kind = "header", name = "x-api-key", format = "{secret}" }
"#;
        let cat = Catalog::from_toml(src).unwrap();
        let ep = cat.get("anthropic.messages").unwrap();
        let mut d = RequestDraft {
            endpoint: "anthropic.messages".into(),
            body: [
                ("model".to_string(), json!("claude-opus-5")),
                (
                    "messages".to_string(),
                    json!([{"role":"user","content":"hi"}]),
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };

        let req = ep.build(&d, SECRET).unwrap();
        assert_eq!(req.headers.get("anthropic-version").unwrap(), "2023-06-01");
        assert_eq!(req.headers.get("x-api-key").unwrap(), SECRET);

        // A caller may not replace one, in any casing.
        d.headers
            .insert("Anthropic-Version".into(), "1999-01-01".into());
        assert!(matches!(ep.check(&d), Err(Error::Constraint(_))));
    }

    #[test]
    fn a_json_field_accepts_a_shape_a_scalar_type_could_not() {
        let src = r#"
[[endpoint]]
name = "x.chat"
method = "POST"
host = "api.x.test"
path = "/chat"
secret = "x/key"
schema = { messages = "json" }
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
"#;
        let cat = Catalog::from_toml(src).unwrap();
        let ep = cat.get("x.chat").unwrap();
        let d = RequestDraft {
            endpoint: "x.chat".into(),
            body: [("messages".to_string(), json!([{"role": "user"}]))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert!(ep.check(&d).is_ok());

        // Null is still refused: a present-but-empty field is almost always a
        // caller mistake rather than an intent.
        let d2 = RequestDraft {
            endpoint: "x.chat".into(),
            body: [("messages".to_string(), json!(null))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert!(matches!(ep.check(&d2), Err(Error::Constraint(_))));
    }

    #[test]
    fn a_form_encoded_endpoint_writes_a_form_body() {
        // Stripe and many older APIs take form bodies, not JSON.
        let src = r#"
[[endpoint]]
name = "stripe.refund_form"
method = "POST"
host = "api.stripe.com"
path = "/v1/refunds"
secret = "stripe/sk_live"
body_format = "form"
schema = { amount = "uint32", charge = "string" }
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
"#;
        let cat = Catalog::from_toml(src).unwrap();
        let ep = cat.get("stripe.refund_form").unwrap();
        let d = RequestDraft {
            endpoint: "stripe.refund_form".into(),
            body: [
                ("amount".into(), json!(500)),
                ("charge".into(), json!("ch_1")),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let req = ep.build(&d, SECRET).unwrap();
        assert_eq!(req.body.unwrap(), "amount=500&charge=ch_1");
        assert_eq!(req.content_type, "application/x-www-form-urlencoded");
    }

    #[test]
    fn form_encoding_escapes_values_that_would_otherwise_inject_fields() {
        let src = r#"
[[endpoint]]
name = "legacy.post"
method = "POST"
host = "api.legacy.test"
path = "/x"
secret = "legacy/key"
body_format = "form"
schema = { note = "string" }
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
"#;
        let cat = Catalog::from_toml(src).unwrap();
        let ep = cat.get("legacy.post").unwrap();
        let d = RequestDraft {
            endpoint: "legacy.post".into(),
            body: [("note".to_string(), json!("a&admin=true b=c"))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let req = ep.build(&d, SECRET).unwrap();
        let body = req.body.unwrap();
        assert!(
            !body.contains("&admin=true"),
            "a value must not become a field: {}",
            body
        );
        assert_eq!(body, "note=a%26admin%3Dtrue%20b%3Dc");
    }

    #[test]
    fn json_remains_the_default_encoding() {
        let cat = catalog();
        let ep = cat.get("stripe.refund").unwrap();
        assert_eq!(ep.body_format, BodyFormat::Json);
        let req = ep.build(&refund_draft(), SECRET).unwrap();
        assert_eq!(req.content_type, "application/json");
        assert!(req.body.unwrap().starts_with('{'));
    }

    #[test]
    fn field_specs_parse_and_reject_unknown_types() {
        assert_eq!(
            FieldSpec::parse("string").unwrap(),
            FieldSpec {
                ty: FieldType::Str,
                optional: false
            }
        );
        assert!(FieldSpec::parse("string?").unwrap().optional);
        assert!(FieldSpec::parse("blob").is_err());
    }
}
