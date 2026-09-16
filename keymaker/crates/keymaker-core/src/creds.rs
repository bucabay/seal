//! Approach 3 — make the stolen value worthless.
//!
//! Rather than trying to keep an injected value in, mint one that expires. A
//! credential valid for five minutes and scoped to one operation makes every
//! exfiltration channel irrelevant without having to close any of them.
//!
//! This only works where the provider will mint short-lived credentials. A
//! static API key or a database password has nothing to shorten, and for those
//! the broker (Approach 2) is the only answer. [`Source::acquire`] reports
//! which of the two happened, so the difference is never silently glossed.

use crate::clock::Clock;
use crate::error::{Error, Result};
use crate::store::{Secret, SecretStore};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeRequest {
    /// Which stored credential to exchange.
    pub base_ref: String,
    /// Permissions the minted credential should carry.
    pub scopes: Vec<String>,
    /// Who it is for, e.g. `api.stripe.com`.
    pub audience: String,
    /// Requested lifetime in seconds.
    pub ttl: u64,
}

impl ExchangeRequest {
    pub fn new(base_ref: impl Into<String>, audience: impl Into<String>, ttl: u64) -> Self {
        ExchangeRequest {
            base_ref: base_ref.into(),
            scopes: Vec::new(),
            audience: audience.into(),
            ttl,
        }
    }

    pub fn with_scopes<S: Into<String>>(mut self, scopes: Vec<S>) -> Self {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Debug, Clone)]
pub struct ShortLived {
    pub value: Secret,
    pub expires_at: u64,
    pub scopes: Vec<String>,
    pub audience: String,
    /// Extra pieces, for credentials that are not a single string. AWS returns
    /// a key id, a secret and a session token; all three are secret and all
    /// three must be redacted, so they live here rather than being flattened
    /// into `value`.
    pub parts: std::collections::BTreeMap<String, Secret>,
}

impl ShortLived {
    /// Every secret string in this credential, for the redactor.
    pub fn all_values(&self) -> Vec<String> {
        let mut out = vec![self.value.expose().to_string()];
        out.extend(self.parts.values().map(|p| p.expose().to_string()));
        out
    }
}

impl ShortLived {
    pub fn valid_at(&self, now: u64) -> bool {
        now < self.expires_at
    }

    pub fn seconds_left(&self, now: u64) -> u64 {
        self.expires_at.saturating_sub(now)
    }
}

/// How a credential was obtained. The distinction matters enough to be in the
/// type: only one of these expires.
#[derive(Debug, Clone)]
pub enum Acquired {
    /// Minted for this call, and short-lived.
    Minted(ShortLived),
    /// No exchange available: the stored value is being used as-is. Anything
    /// that captures it keeps it.
    Static(Secret),
}

impl Acquired {
    pub fn value(&self) -> &Secret {
        match self {
            Acquired::Minted(s) => &s.value,
            Acquired::Static(s) => s,
        }
    }

    pub fn is_ephemeral(&self) -> bool {
        matches!(self, Acquired::Minted(_))
    }

    /// A line to show the user when a static credential had to be used.
    pub fn warning(&self) -> Option<String> {
        match self {
            Acquired::Minted(_) => None,
            Acquired::Static(_) => Some(
                "no short-lived credential available; a static value was used and will \
                 remain valid if it escapes"
                    .into(),
            ),
        }
    }
}

/// Turns a long-lived credential into a narrow, short-lived one.
pub trait Exchanger: std::fmt::Debug {
    /// Whether this exchanger handles the given reference at all.
    fn handles(&self, base_ref: &str) -> bool;
    fn exchange(&self, base: &Secret, req: &ExchangeRequest, now: u64) -> Result<ShortLived>;
}

/// Chooses an exchanger, falling back to the stored value.
#[derive(Debug)]
pub struct Source<'a> {
    store: &'a dyn SecretStore,
    clock: &'a dyn Clock,
    exchangers: Vec<Box<dyn Exchanger + 'a>>,
    /// Refuse to fall back to a static value. Off by default, because most
    /// real credentials cannot be exchanged yet.
    pub require_ephemeral: bool,
}

impl<'a> Source<'a> {
    pub fn new(store: &'a dyn SecretStore, clock: &'a dyn Clock) -> Self {
        Source {
            store,
            clock,
            exchangers: Vec::new(),
            require_ephemeral: false,
        }
    }

    pub fn with_exchanger(mut self, e: Box<dyn Exchanger + 'a>) -> Self {
        self.exchangers.push(e);
        self
    }

    pub fn requiring_ephemeral(mut self) -> Self {
        self.require_ephemeral = true;
        self
    }

    pub fn acquire(&self, req: &ExchangeRequest) -> Result<Acquired> {
        let base = self.store.get(&req.base_ref)?;
        let now = self.clock.now();
        for e in &self.exchangers {
            if e.handles(&req.base_ref) {
                let minted = e.exchange(&base, req, now)?;
                if !minted.valid_at(now) {
                    return Err(Error::Store(
                        "exchanger returned an already-expired credential".into(),
                    ));
                }
                return Ok(Acquired::Minted(minted));
            }
        }
        if self.require_ephemeral {
            return Err(Error::Denied(format!(
                "`{}` has no exchanger and ephemeral credentials are required",
                req.base_ref
            )));
        }
        Ok(Acquired::Static(base))
    }
}

/// An exchanger for tests and for documenting the shape a real one takes.
#[derive(Debug, Clone)]
pub struct MockExchanger {
    pub prefix: String,
    /// Cap applied to whatever TTL is requested, as every real STS does.
    pub max_ttl: u64,
}

impl MockExchanger {
    pub fn new(prefix: impl Into<String>, max_ttl: u64) -> Self {
        MockExchanger {
            prefix: prefix.into(),
            max_ttl,
        }
    }
}

impl Exchanger for MockExchanger {
    fn handles(&self, base_ref: &str) -> bool {
        base_ref.starts_with(&self.prefix)
    }

    fn exchange(&self, base: &Secret, req: &ExchangeRequest, now: u64) -> Result<ShortLived> {
        let ttl = req.ttl.min(self.max_ttl).max(1);
        Ok(ShortLived {
            value: Secret::new(format!(
                "tmp_{}_{}",
                &base.expose()[..3.min(base.len())],
                now
            )),
            expires_at: now + ttl,
            scopes: req.scopes.clone(),
            audience: req.audience.clone(),
            parts: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::store::MemoryStore;

    fn store() -> MemoryStore {
        MemoryStore::with([
            ("aws/root", "AKIAROOT"),
            ("stripe/sk_live", "sk_live_static"),
        ])
    }

    #[test]
    fn a_credential_with_no_exchanger_comes_back_static_and_warns() {
        let s = store();
        let c = FixedClock::new(1_000);
        let src = Source::new(&s, &c);
        let got = src
            .acquire(&ExchangeRequest::new(
                "stripe/sk_live",
                "api.stripe.com",
                300,
            ))
            .unwrap();

        assert!(!got.is_ephemeral());
        assert_eq!(got.value().expose(), "sk_live_static");
        assert!(got.warning().is_some(), "a static fallback must say so");
    }

    #[test]
    fn a_credential_with_an_exchanger_is_minted_and_expires() {
        let s = store();
        let c = FixedClock::new(1_000);
        let src = Source::new(&s, &c).with_exchanger(Box::new(MockExchanger::new("aws/", 900)));
        let got = src
            .acquire(&ExchangeRequest::new("aws/root", "sts.amazonaws.com", 300))
            .unwrap();

        assert!(got.is_ephemeral());
        assert!(got.warning().is_none());
        assert_ne!(
            got.value().expose(),
            "AKIAROOT",
            "the root must never be handed out"
        );

        let Acquired::Minted(m) = got else {
            panic!("expected minted")
        };
        assert_eq!(m.expires_at, 1_300);
        assert!(m.valid_at(1_299));
        assert!(!m.valid_at(1_300), "expiry is exclusive");
        assert_eq!(m.seconds_left(1_250), 50);
        assert_eq!(m.seconds_left(9_999), 0, "never goes negative");
    }

    #[test]
    fn an_exchanger_caps_the_requested_lifetime() {
        let s = store();
        let c = FixedClock::new(0);
        let src = Source::new(&s, &c).with_exchanger(Box::new(MockExchanger::new("aws/", 900)));
        let got = src
            .acquire(&ExchangeRequest::new("aws/root", "sts", 86_400))
            .unwrap();
        let Acquired::Minted(m) = got else { panic!() };
        assert_eq!(
            m.expires_at, 900,
            "a caller cannot ask for a longer life than allowed"
        );
    }

    #[test]
    fn scopes_and_audience_travel_with_the_minted_credential() {
        let s = store();
        let c = FixedClock::new(0);
        let src = Source::new(&s, &c).with_exchanger(Box::new(MockExchanger::new("aws/", 900)));
        let req = ExchangeRequest::new("aws/root", "sts.amazonaws.com", 60)
            .with_scopes(vec!["s3:GetObject"]);
        let Acquired::Minted(m) = src.acquire(&req).unwrap() else {
            panic!()
        };
        assert_eq!(m.scopes, vec!["s3:GetObject"]);
        assert_eq!(m.audience, "sts.amazonaws.com");
    }

    #[test]
    fn requiring_ephemeral_refuses_to_fall_back() {
        let s = store();
        let c = FixedClock::new(0);
        let src = Source::new(&s, &c)
            .with_exchanger(Box::new(MockExchanger::new("aws/", 900)))
            .requiring_ephemeral();

        assert!(src
            .acquire(&ExchangeRequest::new("aws/root", "sts", 60))
            .is_ok());
        assert!(
            matches!(
                src.acquire(&ExchangeRequest::new(
                    "stripe/sk_live",
                    "api.stripe.com",
                    60
                )),
                Err(Error::Denied(_))
            ),
            "a static key must be refused when only ephemeral is acceptable"
        );
    }

    #[test]
    fn a_missing_secret_is_reported_before_any_exchange() {
        let s = store();
        let c = FixedClock::new(0);
        let src = Source::new(&s, &c);
        assert!(matches!(
            src.acquire(&ExchangeRequest::new("nope/none", "x", 60)),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn an_exchanger_returning_a_dead_credential_is_rejected() {
        #[derive(Debug)]
        struct Broken;
        impl Exchanger for Broken {
            fn handles(&self, _: &str) -> bool {
                true
            }
            fn exchange(&self, _: &Secret, _: &ExchangeRequest, now: u64) -> Result<ShortLived> {
                Ok(ShortLived {
                    value: Secret::new("dead"),
                    expires_at: now, // already expired
                    scopes: vec![],
                    audience: "x".into(),
                    parts: Default::default(),
                })
            }
        }
        let s = store();
        let c = FixedClock::new(1_000);
        let src = Source::new(&s, &c).with_exchanger(Box::new(Broken));
        assert!(matches!(
            src.acquire(&ExchangeRequest::new("aws/root", "x", 60)),
            Err(Error::Store(_))
        ));
    }

    #[test]
    fn a_minted_credential_does_not_print_itself() {
        let s = store();
        let c = FixedClock::new(0);
        let src = Source::new(&s, &c).with_exchanger(Box::new(MockExchanger::new("aws/", 900)));
        let got = src
            .acquire(&ExchangeRequest::new("aws/root", "sts", 60))
            .unwrap();
        assert!(!format!("{:?}", got).contains("tmp_AKI"));
    }
}

/// OAuth 2.0 Token Exchange — [RFC 8693].
///
/// The generic mechanism: hand a long-lived token to a security token service
/// and receive a narrower, shorter-lived one. Works with anything that
/// implements the RFC — Keycloak, Auth0, Okta, Google, an in-house STS.
///
/// The exchange carries `actor_token` when one is configured, which is what
/// makes "who authorised this" answerable afterwards rather than everything
/// appearing to come from one identity.
///
/// [RFC 8693]: https://www.rfc-editor.org/rfc/rfc8693
#[derive(Debug, Clone)]
pub struct TokenExchanger {
    /// References with this prefix are handled here.
    pub prefix: String,
    /// The STS token endpoint. Must be https.
    pub token_url: String,
    /// Identifies this broker to the STS, when the STS requires it.
    pub client_id: Option<String>,
    /// Records that the broker is acting on behalf of the subject.
    pub actor_token: Option<String>,
}

pub const GRANT_TYPE_TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
pub const TOKEN_TYPE_ACCESS: &str = "urn:ietf:params:oauth:token-type:access_token";

impl TokenExchanger {
    pub fn new(prefix: impl Into<String>, token_url: impl Into<String>) -> Self {
        TokenExchanger {
            prefix: prefix.into(),
            token_url: token_url.into(),
            client_id: None,
            actor_token: None,
        }
    }

    pub fn with_client_id(mut self, id: impl Into<String>) -> Self {
        self.client_id = Some(id.into());
        self
    }

    pub fn with_actor_token(mut self, token: impl Into<String>) -> Self {
        self.actor_token = Some(token.into());
        self
    }

    /// The form body for an exchange. Separated so it can be asserted on
    /// without a network.
    pub fn form(&self, base: &Secret, req: &ExchangeRequest) -> Vec<(String, String)> {
        let mut form = vec![
            (
                "grant_type".to_string(),
                GRANT_TYPE_TOKEN_EXCHANGE.to_string(),
            ),
            ("subject_token".to_string(), base.expose().to_string()),
            (
                "subject_token_type".to_string(),
                TOKEN_TYPE_ACCESS.to_string(),
            ),
            (
                "requested_token_type".to_string(),
                TOKEN_TYPE_ACCESS.to_string(),
            ),
            ("audience".to_string(), req.audience.clone()),
        ];
        if !req.scopes.is_empty() {
            form.push(("scope".to_string(), req.scopes.join(" ")));
        }
        if let Some(id) = &self.client_id {
            form.push(("client_id".to_string(), id.clone()));
        }
        if let Some(actor) = &self.actor_token {
            form.push(("actor_token".to_string(), actor.clone()));
            form.push((
                "actor_token_type".to_string(),
                TOKEN_TYPE_ACCESS.to_string(),
            ));
        }
        form
    }

    /// Read the STS reply.
    ///
    /// `expires_in` is optional in the RFC. When it is absent the lifetime is
    /// unknown, and an unknown lifetime is not a short one — so the requested
    /// TTL is used rather than assuming the token is long-lived.
    pub fn parse_reply(&self, body: &str, req: &ExchangeRequest, now: u64) -> Result<ShortLived> {
        let v: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| Error::Store(format!("token endpoint returned invalid JSON: {}", e)))?;

        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            let desc = v
                .get("error_description")
                .and_then(|d| d.as_str())
                .unwrap_or("no description");
            return Err(Error::Denied(format!(
                "token exchange refused: {} ({})",
                err, desc
            )));
        }

        let token = v
            .get("access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| Error::Store("token endpoint returned no access_token".into()))?;
        if token.is_empty() {
            return Err(Error::Store(
                "token endpoint returned an empty access_token".into(),
            ));
        }

        let ttl = v
            .get("expires_in")
            .and_then(|e| e.as_u64())
            .unwrap_or(req.ttl)
            .max(1);

        // Scope may come back narrower than asked for; record what was granted,
        // not what was wanted.
        let scopes = v
            .get("scope")
            .and_then(|s| s.as_str())
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_else(|| req.scopes.clone());

        Ok(ShortLived {
            value: Secret::new(token),
            expires_at: now + ttl,
            scopes,
            audience: req.audience.clone(),
            parts: Default::default(),
        })
    }
}

impl Exchanger for TokenExchanger {
    fn handles(&self, base_ref: &str) -> bool {
        base_ref.starts_with(&self.prefix)
    }

    fn exchange(&self, _base: &Secret, _req: &ExchangeRequest, _now: u64) -> Result<ShortLived> {
        // The network call lives in `HttpTokenExchanger`, which wraps this with
        // a transport. Keeping the protocol logic here means it is testable
        // without one.
        Err(Error::Store(
            "TokenExchanger needs a transport; use HttpTokenExchanger".into(),
        ))
    }
}

/// [`TokenExchanger`] plus a way to actually reach the STS.
///
/// Split from the protocol above so that the wire format is testable without a
/// network, and so the only code that puts a long-lived token on a socket is
/// this one small wrapper.
#[derive(Debug)]
pub struct HttpTokenExchanger<'a> {
    pub inner: TokenExchanger,
    transport: &'a dyn crate::broker::Transport,
}

impl<'a> HttpTokenExchanger<'a> {
    pub fn new(inner: TokenExchanger, transport: &'a dyn crate::broker::Transport) -> Self {
        HttpTokenExchanger { inner, transport }
    }
}

fn form_encode(pairs: &[(String, String)]) -> String {
    fn esc(s: &str) -> String {
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
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", esc(k), esc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

impl Exchanger for HttpTokenExchanger<'_> {
    fn handles(&self, base_ref: &str) -> bool {
        self.inner.handles(base_ref)
    }

    fn exchange(&self, base: &Secret, req: &ExchangeRequest, now: u64) -> Result<ShortLived> {
        // The subject token is a live credential, so the same rule as every
        // other request applies: never over plain http.
        if !self.inner.token_url.starts_with("https://") {
            return Err(Error::Constraint(format!(
                "refusing to send a token to a non-https endpoint: {}",
                self.inner.token_url
            )));
        }

        let body = form_encode(&self.inner.form(base, req));
        let prepared = crate::provider::PreparedRequest {
            method: "POST".into(),
            url: self.inner.token_url.clone(),
            headers: std::collections::BTreeMap::from([(
                "Accept".to_string(),
                "application/json".to_string(),
            )]),
            body: Some(body),
            content_type: "application/x-www-form-urlencoded",
        };

        let resp = self.transport.send(&prepared)?;
        if resp.status >= 500 {
            return Err(Error::Store(format!(
                "token endpoint returned {}",
                resp.status
            )));
        }
        self.inner.parse_reply(&resp.body, req, now)
    }
}

#[cfg(test)]
mod exchange_tests {
    use super::*;
    use crate::broker::{HttpResponse, Transport};
    use crate::clock::FixedClock;
    use crate::provider::PreparedRequest;
    use crate::store::MemoryStore;
    use std::cell::RefCell;

    #[derive(Debug)]
    struct FakeSts {
        reply: HttpResponse,
        sent: RefCell<Vec<PreparedRequest>>,
    }

    impl FakeSts {
        fn returning(body: &str) -> Self {
            FakeSts {
                reply: HttpResponse {
                    status: 200,
                    body: body.into(),
                },
                sent: RefCell::new(Vec::new()),
            }
        }
        fn with_status(status: u16, body: &str) -> Self {
            FakeSts {
                reply: HttpResponse {
                    status,
                    body: body.into(),
                },
                sent: RefCell::new(Vec::new()),
            }
        }
        fn last_form(&self) -> Vec<(String, String)> {
            let sent = self.sent.borrow();
            let body = sent.last().unwrap().body.clone().unwrap();
            body.split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (decode(k), decode(v)))
                .collect()
        }
    }

    fn decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    impl Transport for FakeSts {
        fn send(&self, req: &PreparedRequest) -> Result<HttpResponse> {
            self.sent.borrow_mut().push(req.clone());
            Ok(self.reply.clone())
        }
    }

    fn store() -> MemoryStore {
        MemoryStore::with([("sts/subject", "long-lived-subject-token")])
    }

    fn request() -> ExchangeRequest {
        ExchangeRequest::new("sts/subject", "https://api.example.com", 300)
            .with_scopes(vec!["read:things", "write:things"])
    }

    fn exchanger(sts: &FakeSts) -> HttpTokenExchanger<'_> {
        HttpTokenExchanger::new(
            TokenExchanger::new("sts/", "https://sts.example.com/token").with_client_id("keymaker"),
            sts,
        )
    }

    #[test]
    fn an_exchange_sends_the_rfc_8693_form() {
        let sts = FakeSts::returning(r#"{"access_token":"short","expires_in":300}"#);
        let c = FixedClock::new(1_000);
        let s = store();
        let src = Source::new(&s, &c).with_exchanger(Box::new(exchanger(&sts)));

        src.acquire(&request()).unwrap();

        let form: std::collections::BTreeMap<String, String> =
            sts.last_form().into_iter().collect();
        assert_eq!(form["grant_type"], GRANT_TYPE_TOKEN_EXCHANGE);
        assert_eq!(form["subject_token"], "long-lived-subject-token");
        assert_eq!(form["subject_token_type"], TOKEN_TYPE_ACCESS);
        assert_eq!(form["audience"], "https://api.example.com");
        assert_eq!(form["scope"], "read:things write:things");
        assert_eq!(form["client_id"], "keymaker");

        let sent = sts.sent.borrow();
        assert_eq!(sent[0].method, "POST");
        assert_eq!(sent[0].content_type, "application/x-www-form-urlencoded");
    }

    #[test]
    fn the_minted_token_replaces_the_subject_and_expires() {
        let sts = FakeSts::returning(r#"{"access_token":"short-lived","expires_in":120}"#);
        let c = FixedClock::new(1_000);
        let s = store();
        let src = Source::new(&s, &c).with_exchanger(Box::new(exchanger(&sts)));

        let got = src.acquire(&request()).unwrap();
        assert!(got.is_ephemeral());
        assert_eq!(got.value().expose(), "short-lived");
        assert_ne!(
            got.value().expose(),
            "long-lived-subject-token",
            "the subject token must never be handed on"
        );

        let Acquired::Minted(m) = got else { panic!() };
        assert_eq!(m.expires_at, 1_120);
    }

    #[test]
    fn an_actor_token_is_included_so_delegation_is_recorded() {
        let sts = FakeSts::returning(r#"{"access_token":"x","expires_in":60}"#);
        let c = FixedClock::new(0);
        let s = store();
        let ex = HttpTokenExchanger::new(
            TokenExchanger::new("sts/", "https://sts.example.com/token")
                .with_actor_token("broker-identity"),
            &sts,
        );
        let src = Source::new(&s, &c).with_exchanger(Box::new(ex));
        src.acquire(&request()).unwrap();

        let form: std::collections::BTreeMap<String, String> =
            sts.last_form().into_iter().collect();
        assert_eq!(form["actor_token"], "broker-identity");
        assert_eq!(form["actor_token_type"], TOKEN_TYPE_ACCESS);
    }

    #[test]
    fn a_narrower_granted_scope_is_recorded_rather_than_the_one_asked_for() {
        let sts =
            FakeSts::returning(r#"{"access_token":"x","expires_in":60,"scope":"read:things"}"#);
        let c = FixedClock::new(0);
        let s = store();
        let src = Source::new(&s, &c).with_exchanger(Box::new(exchanger(&sts)));

        let Acquired::Minted(m) = src.acquire(&request()).unwrap() else {
            panic!()
        };
        assert_eq!(
            m.scopes,
            vec!["read:things"],
            "what the STS granted, not what was requested"
        );
    }

    #[test]
    fn a_missing_expiry_does_not_become_an_assumed_long_life() {
        // `expires_in` is optional in the RFC. Unknown is not the same as long.
        let sts = FakeSts::returning(r#"{"access_token":"x"}"#);
        let c = FixedClock::new(1_000);
        let s = store();
        let src = Source::new(&s, &c).with_exchanger(Box::new(exchanger(&sts)));

        let Acquired::Minted(m) = src.acquire(&request()).unwrap() else {
            panic!()
        };
        assert_eq!(m.expires_at, 1_300, "falls back to the requested ttl");
    }

    #[test]
    fn an_oauth_error_is_surfaced_as_a_refusal() {
        let sts = FakeSts::with_status(
            400,
            r#"{"error":"invalid_scope","error_description":"write:things not permitted"}"#,
        );
        let c = FixedClock::new(0);
        let s = store();
        let src = Source::new(&s, &c).with_exchanger(Box::new(exchanger(&sts)));

        match src.acquire(&request()) {
            Err(Error::Denied(m)) => {
                assert!(m.contains("invalid_scope"));
                assert!(m.contains("not permitted"));
            }
            other => panic!("expected a refusal, got {:?}", other),
        }
    }

    #[test]
    fn a_broken_reply_is_an_error_rather_than_a_mystery_credential() {
        let c = FixedClock::new(0);
        let s = store();
        for body in [r#"{"nope":1}"#, "not json", r#"{"access_token":""}"#] {
            let sts = FakeSts::returning(body);
            let src = Source::new(&s, &c).with_exchanger(Box::new(exchanger(&sts)));
            assert!(
                src.acquire(&request()).is_err(),
                "`{}` should not produce a credential",
                body
            );
        }
    }

    #[test]
    fn a_server_error_is_not_mistaken_for_a_token() {
        let sts = FakeSts::with_status(503, "gateway down");
        let c = FixedClock::new(0);
        let s = store();
        let src = Source::new(&s, &c).with_exchanger(Box::new(exchanger(&sts)));
        assert!(matches!(src.acquire(&request()), Err(Error::Store(_))));
    }

    #[test]
    fn a_subject_token_is_never_sent_over_plain_http() {
        let sts = FakeSts::returning(r#"{"access_token":"x"}"#);
        let c = FixedClock::new(0);
        let s = store();
        let ex = HttpTokenExchanger::new(
            TokenExchanger::new("sts/", "http://sts.example.com/token"),
            &sts,
        );
        let src = Source::new(&s, &c).with_exchanger(Box::new(ex));

        assert!(matches!(src.acquire(&request()), Err(Error::Constraint(_))));
        assert!(
            sts.sent.borrow().is_empty(),
            "nothing may be sent to a plain-http endpoint"
        );
    }

    #[test]
    fn a_reference_outside_the_prefix_is_left_alone() {
        let sts = FakeSts::returning(r#"{"access_token":"x"}"#);
        let c = FixedClock::new(0);
        let s = MemoryStore::with([("other/key", "static-value")]);
        let src = Source::new(&s, &c).with_exchanger(Box::new(exchanger(&sts)));

        let got = src
            .acquire(&ExchangeRequest::new("other/key", "https://x", 60))
            .unwrap();
        assert!(!got.is_ephemeral());
        assert!(sts.sent.borrow().is_empty());
    }

    #[test]
    fn the_form_encoder_escapes_separators() {
        let encoded = form_encode(&[("scope".into(), "a b&c=d".into()), ("x".into(), "y".into())]);
        assert_eq!(encoded, "scope=a%20b%26c%3Dd&x=y");
    }
}

/// AWS STS `AssumeRole`.
///
/// The canonical short-lived credential: the broker holds a long-lived access
/// key, and every use exchanges it for a role session that expires. A stolen
/// session is inert within the hour; the key it came from never leaves the
/// broker.
///
/// The request is signed with SigV4 — see [`crate::sigv4`] — because that is
/// the only way STS will issue anything.
#[derive(Debug, Clone)]
pub struct StsExchanger {
    pub prefix: String,
    pub role_arn: String,
    pub session_name: String,
    pub region: String,
    /// Where the long-lived secret access key lives. The access key *id* is
    /// public and is carried here.
    pub access_key_id: String,
}

impl StsExchanger {
    pub fn new(
        prefix: impl Into<String>,
        role_arn: impl Into<String>,
        access_key_id: impl Into<String>,
    ) -> Self {
        StsExchanger {
            prefix: prefix.into(),
            role_arn: role_arn.into(),
            session_name: "keymaker".into(),
            region: "us-east-1".into(),
            access_key_id: access_key_id.into(),
        }
    }

    pub fn in_region(mut self, region: impl Into<String>) -> Self {
        self.region = region.into();
        self
    }

    pub fn as_session(mut self, name: impl Into<String>) -> Self {
        self.session_name = name.into();
        self
    }

    pub fn endpoint(&self) -> String {
        format!("https://sts.{}.amazonaws.com/", self.region)
    }

    /// STS caps `DurationSeconds` at 12 hours and requires at least 15 minutes.
    fn duration(&self, requested: u64) -> u64 {
        requested.clamp(900, 43_200)
    }

    pub fn form(&self, ttl: u64) -> Vec<(String, String)> {
        vec![
            ("Action".into(), "AssumeRole".into()),
            ("Version".into(), "2011-06-15".into()),
            ("RoleArn".into(), self.role_arn.clone()),
            ("RoleSessionName".into(), self.session_name.clone()),
            ("DurationSeconds".into(), self.duration(ttl).to_string()),
        ]
    }

    /// Pull one element out of an STS XML reply.
    ///
    /// A full XML parser would be a dependency and a parsing surface for very
    /// little: the reply shape is fixed and every field is a flat text element.
    fn element<'x>(xml: &'x str, tag: &str) -> Option<&'x str> {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        let start = xml.find(&open)? + open.len();
        let end = xml[start..].find(&close)? + start;
        Some(xml[start..end].trim())
    }

    pub fn parse_reply(&self, xml: &str, req: &ExchangeRequest, now: u64) -> Result<ShortLived> {
        if let Some(message) = Self::element(xml, "Message") {
            return Err(Error::Denied(format!("STS refused: {}", message)));
        }
        let key_id = Self::element(xml, "AccessKeyId")
            .ok_or_else(|| Error::Store("STS reply had no AccessKeyId".into()))?;
        let secret = Self::element(xml, "SecretAccessKey")
            .ok_or_else(|| Error::Store("STS reply had no SecretAccessKey".into()))?;
        let token = Self::element(xml, "SessionToken")
            .ok_or_else(|| Error::Store("STS reply had no SessionToken".into()))?;
        if key_id.is_empty() || secret.is_empty() || token.is_empty() {
            return Err(Error::Store("STS reply was incomplete".into()));
        }

        let mut parts = std::collections::BTreeMap::new();
        parts.insert("AWS_ACCESS_KEY_ID".to_string(), Secret::new(key_id));
        parts.insert("AWS_SECRET_ACCESS_KEY".to_string(), Secret::new(secret));
        parts.insert("AWS_SESSION_TOKEN".to_string(), Secret::new(token));

        Ok(ShortLived {
            value: Secret::new(token),
            expires_at: now + self.duration(req.ttl),
            scopes: vec![self.role_arn.clone()],
            audience: req.audience.clone(),
            parts,
        })
    }
}

/// [`StsExchanger`] with a way to reach STS.
#[derive(Debug)]
pub struct HttpStsExchanger<'a> {
    pub inner: StsExchanger,
    transport: &'a dyn crate::broker::Transport,
    clock: &'a dyn Clock,
}

impl<'a> HttpStsExchanger<'a> {
    pub fn new(
        inner: StsExchanger,
        transport: &'a dyn crate::broker::Transport,
        clock: &'a dyn Clock,
    ) -> Self {
        HttpStsExchanger {
            inner,
            transport,
            clock,
        }
    }

    /// `YYYYMMDDTHHMMSSZ` from a unix timestamp, without pulling in a date
    /// library for one format.
    pub fn amz_date(unix: u64) -> String {
        let days = unix / 86_400;
        let secs = unix % 86_400;
        let (y, m, d) = civil_from_days(days as i64);
        format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            y,
            m,
            d,
            secs / 3600,
            (secs % 3600) / 60,
            secs % 60
        )
    }
}

/// Howard Hinnant's days-from-civil, inverted. Exact for any date this will
/// ever see, and avoids a dependency for one conversion.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl Exchanger for HttpStsExchanger<'_> {
    fn handles(&self, base_ref: &str) -> bool {
        base_ref.starts_with(&self.inner.prefix)
    }

    fn exchange(&self, base: &Secret, req: &ExchangeRequest, now: u64) -> Result<ShortLived> {
        let body = form_encode(&self.inner.form(req.ttl));
        let endpoint = self.inner.endpoint();
        let host = endpoint
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_string();
        let timestamp = Self::amz_date(self.clock.now());

        let mut headers = std::collections::BTreeMap::new();
        headers.insert("host".to_string(), host.clone());
        headers.insert(
            "content-type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        );

        let canonical = crate::sigv4::CanonicalRequest {
            method: "POST".into(),
            path: "/".into(),
            query: vec![],
            headers: headers.clone(),
            payload: body.as_bytes().to_vec(),
        };
        let creds = crate::sigv4::Credentials {
            access_key_id: self.inner.access_key_id.clone(),
            secret_access_key: base.expose().to_string(),
            session_token: None,
        };
        let signed = crate::sigv4::sign(&canonical, &creds, &self.inner.region, "sts", &timestamp);

        let mut send_headers = std::collections::BTreeMap::new();
        send_headers.insert("Authorization".to_string(), signed.authorization);
        send_headers.insert("X-Amz-Date".to_string(), timestamp);

        let prepared = crate::provider::PreparedRequest {
            method: "POST".into(),
            url: endpoint,
            headers: send_headers,
            body: Some(body),
            content_type: "application/x-www-form-urlencoded",
        };

        let resp = self.transport.send(&prepared)?;
        if resp.status >= 500 {
            return Err(Error::Store(format!("STS returned {}", resp.status)));
        }
        self.inner.parse_reply(&resp.body, req, now)
    }
}

#[cfg(test)]
mod sts_tests {
    use super::*;
    use crate::broker::{HttpResponse, Transport};
    use crate::clock::FixedClock;
    use crate::provider::PreparedRequest;
    use crate::store::MemoryStore;
    use std::cell::RefCell;

    const REPLY: &str = r#"<AssumeRoleResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <AssumeRoleResult>
    <Credentials>
      <AccessKeyId>ASIAEXAMPLE</AccessKeyId>
      <SecretAccessKey>tmpSecretValue</SecretAccessKey>
      <SessionToken>tmpSessionToken</SessionToken>
      <Expiration>2026-09-16T12:00:00Z</Expiration>
    </Credentials>
  </AssumeRoleResult>
</AssumeRoleResponse>"#;

    #[derive(Debug)]
    struct FakeAws {
        reply: HttpResponse,
        sent: RefCell<Vec<PreparedRequest>>,
    }
    impl FakeAws {
        fn returning(status: u16, body: &str) -> Self {
            FakeAws {
                reply: HttpResponse {
                    status,
                    body: body.into(),
                },
                sent: RefCell::new(Vec::new()),
            }
        }
    }
    impl Transport for FakeAws {
        fn send(&self, req: &PreparedRequest) -> Result<HttpResponse> {
            self.sent.borrow_mut().push(req.clone());
            Ok(self.reply.clone())
        }
    }

    fn store() -> MemoryStore {
        MemoryStore::with([(
            "aws/root_secret",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        )])
    }

    fn sts() -> StsExchanger {
        StsExchanger::new(
            "aws/",
            "arn:aws:iam::123456789012:role/deploy",
            "AKIDEXAMPLE",
        )
        .in_region("eu-west-1")
    }

    fn req() -> ExchangeRequest {
        ExchangeRequest::new("aws/root_secret", "sts.amazonaws.com", 900)
    }

    #[test]
    fn a_role_session_is_minted_and_the_root_key_never_leaves() {
        let aws = FakeAws::returning(200, REPLY);
        let clock = FixedClock::new(1_440_000_000);
        let s = store();
        let src = Source::new(&s, &clock).with_exchanger(Box::new(HttpStsExchanger::new(
            sts(),
            &aws,
            &clock,
        )));

        let got = src.acquire(&req()).unwrap();
        assert!(got.is_ephemeral());
        assert_eq!(got.value().expose(), "tmpSessionToken");

        let Acquired::Minted(m) = got else { panic!() };
        assert_eq!(m.parts["AWS_ACCESS_KEY_ID"].expose(), "ASIAEXAMPLE");
        assert_eq!(m.parts["AWS_SECRET_ACCESS_KEY"].expose(), "tmpSecretValue");
        assert_eq!(m.parts["AWS_SESSION_TOKEN"].expose(), "tmpSessionToken");
        assert_eq!(m.expires_at, 1_440_000_900);

        // The long-lived key was used to sign and nothing more.
        let sent = aws.sent.borrow();
        let rendered = format!("{:?}", sent[0]);
        assert!(
            !rendered.contains("wJalrXUtnFEMI"),
            "the root secret must never appear in the request"
        );
    }

    #[test]
    fn every_part_of_the_credential_is_offered_for_redaction() {
        let aws = FakeAws::returning(200, REPLY);
        let clock = FixedClock::new(1_440_000_000);
        let s = store();
        let src = Source::new(&s, &clock).with_exchanger(Box::new(HttpStsExchanger::new(
            sts(),
            &aws,
            &clock,
        )));
        let Acquired::Minted(m) = src.acquire(&req()).unwrap() else {
            panic!()
        };

        let values = m.all_values();
        for expected in ["ASIAEXAMPLE", "tmpSecretValue", "tmpSessionToken"] {
            assert!(
                values.iter().any(|v| v == expected),
                "{} must be redactable",
                expected
            );
        }
    }

    #[test]
    fn the_request_is_signed_and_addressed_to_the_right_region() {
        let aws = FakeAws::returning(200, REPLY);
        let clock = FixedClock::new(1_440_000_000);
        let s = store();
        let src = Source::new(&s, &clock).with_exchanger(Box::new(HttpStsExchanger::new(
            sts(),
            &aws,
            &clock,
        )));
        src.acquire(&req()).unwrap();

        let sent = aws.sent.borrow();
        assert_eq!(sent[0].url, "https://sts.eu-west-1.amazonaws.com/");
        let auth = sent[0].headers.get("Authorization").unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"));
        assert!(auth.contains("/eu-west-1/sts/aws4_request"));
        assert!(sent[0].headers.contains_key("X-Amz-Date"));
        assert!(sent[0].body.as_ref().unwrap().contains("Action=AssumeRole"));
    }

    #[test]
    fn the_duration_is_clamped_to_what_sts_accepts() {
        let e = sts();
        assert_eq!(e.form(60)[4].1, "900", "below the minimum");
        assert_eq!(e.form(3_600)[4].1, "3600");
        assert_eq!(e.form(999_999)[4].1, "43200", "above the maximum");
    }

    #[test]
    fn an_sts_refusal_is_reported_rather_than_parsed_as_success() {
        let error = r#"<ErrorResponse><Error><Code>AccessDenied</Code>
            <Message>User is not authorized to perform sts:AssumeRole</Message>
            </Error></ErrorResponse>"#;
        let aws = FakeAws::returning(403, error);
        let clock = FixedClock::new(0);
        let s = store();
        let src = Source::new(&s, &clock).with_exchanger(Box::new(HttpStsExchanger::new(
            sts(),
            &aws,
            &clock,
        )));

        match src.acquire(&req()) {
            Err(Error::Denied(m)) => assert!(m.contains("not authorized")),
            other => panic!("expected a refusal, got {:?}", other),
        }
    }

    #[test]
    fn an_incomplete_reply_is_an_error() {
        let clock = FixedClock::new(0);
        let s = store();
        for body in [
            "<AssumeRoleResponse></AssumeRoleResponse>",
            "<AccessKeyId>ASIA</AccessKeyId>",
            "<AccessKeyId></AccessKeyId><SecretAccessKey></SecretAccessKey><SessionToken></SessionToken>",
        ] {
            let aws = FakeAws::returning(200, body);
            let src = Source::new(&s, &clock)
                .with_exchanger(Box::new(HttpStsExchanger::new(sts(), &aws, &clock)));
            assert!(
                src.acquire(&req()).is_err(),
                "`{}` must not yield a credential",
                body
            );
        }
    }

    #[test]
    fn the_timestamp_format_is_what_sigv4_expects() {
        // 2015-08-30T12:36:00Z, the date from AWS's own test vector.
        assert_eq!(
            HttpStsExchanger::amz_date(1_440_938_160),
            "20150830T123600Z"
        );
        assert_eq!(HttpStsExchanger::amz_date(0), "19700101T000000Z");
        // A leap day, where a naive conversion goes wrong.
        assert_eq!(
            HttpStsExchanger::amz_date(1_709_164_800),
            "20240229T000000Z"
        );
    }
}
