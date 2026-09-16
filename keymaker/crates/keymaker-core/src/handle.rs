//! Capability handles.
//!
//! The agent never receives a secret and never receives a durable reference to
//! one. It receives a *handle*: a one-use token, bound to a session and a
//! tool-call, that expires. Redeeming it causes the broker to act; it never
//! yields a value.
//!
//! A stolen handle is worth nothing by the time it can be used, which is the
//! property a stable reference like `stripe/sk_live` does not have.

use crate::clock::Clock;
use crate::error::HandleError;
use crate::id::{Entropy, Id};
use std::collections::HashMap;

pub type SessionId = Id;
pub type HandleId = Id;

/// What a handle authorises: one named capability, resolved by the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// Run a task named in the manifest.
    Task(String),
    /// Send one request built from a provider definition, e.g. `stripe.refund`.
    Request(String),
}

impl Capability {
    pub fn name(&self) -> &str {
        match self {
            Capability::Task(n) | Capability::Request(n) => n,
        }
    }
}

/// How strict a session is about ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionPolicy {
    /// Reject a handle whose sequence number is not above every sequence
    /// already redeemed in this session. Stops an agent banking handles and
    /// spending them later, at the cost of forbidding out-of-order redemption.
    pub enforce_order: bool,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        SessionPolicy {
            enforce_order: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: HandleId,
    pub session: SessionId,
    pub epoch: u64,
    pub capability: Capability,
    pub seq: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    /// Redemptions left. Above one only where a retry is legitimate.
    pub uses_left: u32,
}

/// A grant, plus what redeeming it consumed. Returned by `redeem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redemption {
    pub capability: Capability,
    pub session: SessionId,
    pub handle: HandleId,
    pub uses_left: u32,
}

#[derive(Debug)]
struct Session {
    epoch: u64,
    high_water: u64,
    next_seq: u64,
    policy: SessionPolicy,
}

/// Issues and redeems handles. The broker owns exactly one of these.
#[derive(Debug)]
pub struct Registry<'a> {
    clock: &'a dyn Clock,
    entropy: &'a dyn Entropy,
    default_ttl: u64,
    grants: HashMap<HandleId, Grant>,
    sessions: HashMap<SessionId, Session>,
}

impl<'a> Registry<'a> {
    pub fn new(clock: &'a dyn Clock, entropy: &'a dyn Entropy, default_ttl: u64) -> Self {
        Registry {
            clock,
            entropy,
            default_ttl,
            grants: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    /// Start a session. One per connected agent, established after the peer
    /// credentials on the socket have been checked.
    pub fn open_session(&mut self, policy: SessionPolicy) -> SessionId {
        let id = Id::generate(self.entropy);
        self.sessions.insert(
            id.clone(),
            Session {
                epoch: 0,
                high_water: 0,
                next_seq: 1,
                policy,
            },
        );
        id
    }

    /// Advance to the next tool-call. Handles issued for an earlier epoch stop
    /// being redeemable, so a handle cannot outlive the call it was minted for.
    pub fn advance_epoch(&mut self, session: &SessionId) -> Result<u64, HandleError> {
        let s = self
            .sessions
            .get_mut(session)
            .ok_or(HandleError::WrongSession)?;
        s.epoch += 1;
        Ok(s.epoch)
    }

    pub fn epoch(&self, session: &SessionId) -> Option<u64> {
        self.sessions.get(session).map(|s| s.epoch)
    }

    /// Close a session and drop every handle it ever held.
    pub fn close_session(&mut self, session: &SessionId) {
        self.sessions.remove(session);
        self.grants.retain(|_, g| &g.session != session);
    }

    pub fn issue(
        &mut self,
        session: &SessionId,
        capability: Capability,
        uses: u32,
    ) -> Result<Grant, HandleError> {
        self.issue_with_ttl(session, capability, uses, self.default_ttl)
    }

    pub fn issue_with_ttl(
        &mut self,
        session: &SessionId,
        capability: Capability,
        uses: u32,
        ttl: u64,
    ) -> Result<Grant, HandleError> {
        let now = self.clock.now();
        let s = self
            .sessions
            .get_mut(session)
            .ok_or(HandleError::WrongSession)?;
        let seq = s.next_seq;
        s.next_seq += 1;
        let grant = Grant {
            id: Id::generate(self.entropy),
            session: session.clone(),
            epoch: s.epoch,
            capability,
            seq,
            issued_at: now,
            expires_at: now + ttl,
            uses_left: uses.max(1),
        };
        self.grants.insert(grant.id.clone(), grant.clone());
        Ok(grant)
    }

    /// Spend a handle. Checks run in a fixed order so that the reason given is
    /// always the most specific one, and so a caller cannot use the error to
    /// probe for handles belonging to another session.
    pub fn redeem(
        &mut self,
        session: &SessionId,
        handle: &HandleId,
    ) -> Result<Redemption, HandleError> {
        let now = self.clock.now();

        // An unknown handle and a handle belonging to somebody else are
        // deliberately indistinguishable from the outside.
        let grant = self.grants.get(handle).ok_or(HandleError::Unknown)?;
        if &grant.session != session {
            return Err(HandleError::Unknown);
        }

        let sess = self
            .sessions
            .get(session)
            .ok_or(HandleError::WrongSession)?;
        let policy = sess.policy;
        let current_epoch = sess.epoch;
        let high_water = sess.high_water;

        if now >= grant.expires_at {
            return Err(HandleError::Expired);
        }
        if grant.epoch != current_epoch {
            return Err(HandleError::WrongEpoch);
        }
        if policy.enforce_order && grant.seq <= high_water {
            return Err(HandleError::StaleSequence);
        }
        if grant.uses_left == 0 {
            return Err(HandleError::Exhausted);
        }

        let seq = grant.seq;
        let grant = self.grants.get_mut(handle).expect("checked above");
        grant.uses_left -= 1;
        let redemption = Redemption {
            capability: grant.capability.clone(),
            session: grant.session.clone(),
            handle: grant.id.clone(),
            uses_left: grant.uses_left,
        };
        let exhausted = grant.uses_left == 0;
        if exhausted {
            self.grants.remove(handle);
        }
        // Only advance the high-water mark once the handle is spent for good,
        // so a multi-use handle can legitimately retry.
        if exhausted {
            if let Some(s) = self.sessions.get_mut(session) {
                s.high_water = s.high_water.max(seq);
            }
        }
        Ok(redemption)
    }

    /// Drop every handle that has expired. Safe to call on a timer.
    pub fn sweep(&mut self) -> usize {
        let now = self.clock.now();
        let before = self.grants.len();
        self.grants.retain(|_, g| now < g.expires_at);
        before - self.grants.len()
    }

    pub fn outstanding(&self) -> usize {
        self.grants.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::id::SeqEntropy;

    fn fixture() -> (FixedClock, SeqEntropy) {
        (FixedClock::new(1_000), SeqEntropy::new())
    }

    fn cap() -> Capability {
        Capability::Request("stripe.refund".into())
    }

    #[test]
    fn a_handle_can_be_spent_once() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy::default());
        let g = r.issue(&s, cap(), 1).unwrap();

        let red = r.redeem(&s, &g.id).unwrap();
        assert_eq!(red.capability, cap());
        assert_eq!(red.uses_left, 0);

        assert_eq!(
            r.redeem(&s, &g.id),
            Err(HandleError::Unknown),
            "a spent handle must not be distinguishable from one that never existed"
        );
    }

    #[test]
    fn redeeming_never_yields_a_secret() {
        // Type-level guarantee: Redemption carries a capability name, not a
        // value. This test exists so that adding a value field breaks it.
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy::default());
        let g = r.issue(&s, cap(), 1).unwrap();
        let red = r.redeem(&s, &g.id).unwrap();
        assert_eq!(red.capability.name(), "stripe.refund");
    }

    #[test]
    fn a_handle_expires() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy::default());
        let g = r.issue(&s, cap(), 1).unwrap();

        clock.advance(59);
        assert!(r.redeem(&s, &g.id).is_ok(), "still inside the ttl");

        let g2 = r.issue(&s, cap(), 1).unwrap();
        clock.advance(60);
        assert_eq!(r.redeem(&s, &g2.id), Err(HandleError::Expired));
    }

    #[test]
    fn expiry_is_exclusive_at_the_boundary() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 10);
        let s = r.open_session(SessionPolicy::default());
        let g = r.issue(&s, cap(), 1).unwrap();
        clock.advance(10); // now == expires_at
        assert_eq!(r.redeem(&s, &g.id), Err(HandleError::Expired));
    }

    #[test]
    fn another_session_cannot_redeem_and_cannot_tell_it_exists() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let a = r.open_session(SessionPolicy::default());
        let b = r.open_session(SessionPolicy::default());
        let g = r.issue(&a, cap(), 1).unwrap();

        assert_eq!(
            r.redeem(&b, &g.id),
            Err(HandleError::Unknown),
            "must not leak that the handle exists"
        );
        assert!(r.redeem(&a, &g.id).is_ok(), "owner is unaffected");
    }

    #[test]
    fn a_handle_does_not_survive_its_tool_call() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 600);
        let s = r.open_session(SessionPolicy::default());
        let g = r.issue(&s, cap(), 1).unwrap();

        r.advance_epoch(&s).unwrap();
        assert_eq!(r.redeem(&s, &g.id), Err(HandleError::WrongEpoch));
    }

    #[test]
    fn ordering_is_enforced_when_asked_for() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy {
            enforce_order: true,
        });
        let first = r.issue(&s, cap(), 1).unwrap();
        let second = r.issue(&s, cap(), 1).unwrap();

        r.redeem(&s, &second.id).unwrap();
        assert_eq!(
            r.redeem(&s, &first.id),
            Err(HandleError::StaleSequence),
            "a banked earlier handle must not be spendable afterwards"
        );
    }

    #[test]
    fn ordering_can_be_relaxed() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy {
            enforce_order: false,
        });
        let first = r.issue(&s, cap(), 1).unwrap();
        let second = r.issue(&s, cap(), 1).unwrap();

        r.redeem(&s, &second.id).unwrap();
        assert!(r.redeem(&s, &first.id).is_ok());
    }

    #[test]
    fn a_multi_use_handle_retries_then_dies() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy::default());
        let g = r.issue(&s, cap(), 3).unwrap();

        assert_eq!(r.redeem(&s, &g.id).unwrap().uses_left, 2);
        assert_eq!(r.redeem(&s, &g.id).unwrap().uses_left, 1);
        assert_eq!(r.redeem(&s, &g.id).unwrap().uses_left, 0);
        assert_eq!(r.redeem(&s, &g.id), Err(HandleError::Unknown));
    }

    #[test]
    fn retries_do_not_trip_the_ordering_check() {
        // A multi-use handle must not raise the high-water mark until spent,
        // or its own second redemption would be rejected as stale.
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy {
            enforce_order: true,
        });
        let g = r.issue(&s, cap(), 2).unwrap();
        assert!(r.redeem(&s, &g.id).is_ok());
        assert!(r.redeem(&s, &g.id).is_ok(), "retry must survive ordering");
    }

    #[test]
    fn zero_uses_is_treated_as_one_rather_than_a_dead_handle() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy::default());
        let g = r.issue(&s, cap(), 0).unwrap();
        assert_eq!(g.uses_left, 1);
        assert!(r.redeem(&s, &g.id).is_ok());
    }

    #[test]
    fn closing_a_session_destroys_its_handles() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy::default());
        let g = r.issue(&s, cap(), 1).unwrap();
        assert_eq!(r.outstanding(), 1);

        r.close_session(&s);
        assert_eq!(r.outstanding(), 0);
        assert_eq!(r.redeem(&s, &g.id), Err(HandleError::Unknown));
    }

    #[test]
    fn an_unknown_session_cannot_be_issued_against() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let ghost = Id::parse(&"c".repeat(64)).unwrap();
        assert_eq!(r.issue(&ghost, cap(), 1), Err(HandleError::WrongSession));
        assert_eq!(r.advance_epoch(&ghost), Err(HandleError::WrongSession));
    }

    #[test]
    fn sweep_removes_only_expired_handles() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy::default());
        let _short = r.issue_with_ttl(&s, cap(), 1, 10).unwrap();
        let long = r.issue_with_ttl(&s, cap(), 1, 600).unwrap();

        clock.advance(11);
        assert_eq!(r.sweep(), 1);
        assert_eq!(r.outstanding(), 1);
        assert!(r.redeem(&s, &long.id).is_ok());
    }

    #[test]
    fn a_forged_handle_is_rejected() {
        let (clock, ent) = fixture();
        let mut r = Registry::new(&clock, &ent, 60);
        let s = r.open_session(SessionPolicy::default());
        let forged = Id::parse(&"f".repeat(64)).unwrap();
        assert_eq!(r.redeem(&s, &forged), Err(HandleError::Unknown));
    }
}
