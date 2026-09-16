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
        Source { store, clock, exchangers: Vec::new(), require_ephemeral: false }
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
        MockExchanger { prefix: prefix.into(), max_ttl }
    }
}

impl Exchanger for MockExchanger {
    fn handles(&self, base_ref: &str) -> bool {
        base_ref.starts_with(&self.prefix)
    }

    fn exchange(&self, base: &Secret, req: &ExchangeRequest, now: u64) -> Result<ShortLived> {
        let ttl = req.ttl.min(self.max_ttl).max(1);
        Ok(ShortLived {
            value: Secret::new(format!("tmp_{}_{}", &base.expose()[..3.min(base.len())], now)),
            expires_at: now + ttl,
            scopes: req.scopes.clone(),
            audience: req.audience.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::store::MemoryStore;

    fn store() -> MemoryStore {
        MemoryStore::with([("aws/root", "AKIAROOT"), ("stripe/sk_live", "sk_live_static")])
    }

    #[test]
    fn a_credential_with_no_exchanger_comes_back_static_and_warns() {
        let s = store();
        let c = FixedClock::new(1_000);
        let src = Source::new(&s, &c);
        let got = src.acquire(&ExchangeRequest::new("stripe/sk_live", "api.stripe.com", 300)).unwrap();

        assert!(!got.is_ephemeral());
        assert_eq!(got.value().expose(), "sk_live_static");
        assert!(got.warning().is_some(), "a static fallback must say so");
    }

    #[test]
    fn a_credential_with_an_exchanger_is_minted_and_expires() {
        let s = store();
        let c = FixedClock::new(1_000);
        let src = Source::new(&s, &c).with_exchanger(Box::new(MockExchanger::new("aws/", 900)));
        let got = src.acquire(&ExchangeRequest::new("aws/root", "sts.amazonaws.com", 300)).unwrap();

        assert!(got.is_ephemeral());
        assert!(got.warning().is_none());
        assert_ne!(got.value().expose(), "AKIAROOT", "the root must never be handed out");

        let Acquired::Minted(m) = got else { panic!("expected minted") };
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
        let got = src.acquire(&ExchangeRequest::new("aws/root", "sts", 86_400)).unwrap();
        let Acquired::Minted(m) = got else { panic!() };
        assert_eq!(m.expires_at, 900, "a caller cannot ask for a longer life than allowed");
    }

    #[test]
    fn scopes_and_audience_travel_with_the_minted_credential() {
        let s = store();
        let c = FixedClock::new(0);
        let src = Source::new(&s, &c).with_exchanger(Box::new(MockExchanger::new("aws/", 900)));
        let req = ExchangeRequest::new("aws/root", "sts.amazonaws.com", 60)
            .with_scopes(vec!["s3:GetObject"]);
        let Acquired::Minted(m) = src.acquire(&req).unwrap() else { panic!() };
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

        assert!(src.acquire(&ExchangeRequest::new("aws/root", "sts", 60)).is_ok());
        assert!(
            matches!(
                src.acquire(&ExchangeRequest::new("stripe/sk_live", "api.stripe.com", 60)),
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
        let got = src.acquire(&ExchangeRequest::new("aws/root", "sts", 60)).unwrap();
        assert!(!format!("{:?}", got).contains("tmp_AKI"));
    }
}
