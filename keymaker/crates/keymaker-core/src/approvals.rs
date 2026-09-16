//! Human approval for gated capabilities.
//!
//! The broker and the GUI are separate processes, so a decision has to pass
//! between them. It goes through a file rather than a new protocol: the broker
//! records that something is waiting, a person answers in the GUI, and the
//! broker picks the answer up on the next attempt.
//!
//! Three rules make this safe to leave lying on disk:
//!
//! - A decision is **spent** when it is taken. One approval authorises one
//!   call, never a standing permission.
//! - A request **expires**. An approval granted this morning must not still be
//!   sitting there this afternoon waiting for an agent to use it.
//! - The file holds capability *names* and decisions. There is nothing secret
//!   in it, so a wrong file mode is an annoyance rather than a disclosure.

use crate::clock::Clock;
use crate::id::{Entropy, Id};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How long a request waits for an answer, and how long an answer stays good.
pub const DEFAULT_TTL: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pending,
    Granted,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    /// The endpoint or task needing a decision.
    pub capability: String,
    /// The policy rule that stopped it, so the person can see why.
    pub rule: String,
    /// A short description of what is being asked for, with no values in it.
    #[serde(default)]
    pub detail: String,
    pub requested_at: u64,
    pub expires_at: u64,
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<u64>,
}

impl Request {
    pub fn is_live(&self, now: u64) -> bool {
        now < self.expires_at
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    requests: Vec<Request>,
}

/// The shared approval queue.
#[derive(Debug)]
pub struct Approvals<'a> {
    path: PathBuf,
    clock: &'a dyn Clock,
    entropy: &'a dyn Entropy,
    ttl: u64,
}

impl<'a> Approvals<'a> {
    pub fn new(path: impl Into<PathBuf>, clock: &'a dyn Clock, entropy: &'a dyn Entropy) -> Self {
        Approvals {
            path: path.into(),
            clock,
            entropy,
            ttl: DEFAULT_TTL,
        }
    }

    pub fn with_ttl(mut self, ttl: u64) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> File {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn store(&self, file: &File) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(file) {
            // A failed write means the decision is simply not recorded, which
            // fails closed: the call stays blocked.
            let _ = std::fs::write(&self.path, text);
        }
    }

    /// Drop anything that has expired. Called on every operation, so a stale
    /// approval cannot be used later no matter which side looks first.
    fn sweep(&self, file: &mut File) {
        let now = self.clock.now();
        file.requests.retain(|r| r.is_live(now));
    }

    /// Record that something is waiting for a person. Returns the request id.
    ///
    /// An existing live request for the same capability is reused rather than
    /// stacked, so an agent retrying in a loop cannot fill the queue.
    pub fn request(&self, capability: &str, rule: &str, detail: &str) -> String {
        let now = self.clock.now();
        let mut file = self.load();
        self.sweep(&mut file);

        if let Some(existing) = file
            .requests
            .iter()
            .find(|r| r.capability == capability && r.verdict == Verdict::Pending)
        {
            let id = existing.id.clone();
            self.store(&file);
            return id;
        }

        let id = Id::generate(self.entropy).to_string();
        file.requests.push(Request {
            id: id.clone(),
            capability: capability.to_string(),
            rule: rule.to_string(),
            detail: detail.to_string(),
            requested_at: now,
            expires_at: now + self.ttl,
            verdict: Verdict::Pending,
            decided_at: None,
        });
        self.store(&file);
        id
    }

    /// Everything still waiting for an answer.
    pub fn pending(&self) -> Vec<Request> {
        let mut file = self.load();
        self.sweep(&mut file);
        self.store(&file);
        file.requests
            .into_iter()
            .filter(|r| r.verdict == Verdict::Pending)
            .collect()
    }

    /// Everything live, answered or not — what the GUI lists.
    pub fn all(&self) -> Vec<Request> {
        let mut file = self.load();
        self.sweep(&mut file);
        file.requests
    }

    /// A person answers. Returns false if there was nothing live to answer.
    pub fn decide(&self, id: &str, granted: bool) -> bool {
        let now = self.clock.now();
        let mut file = self.load();
        self.sweep(&mut file);

        let Some(req) = file.requests.iter_mut().find(|r| r.id == id) else {
            self.store(&file);
            return false;
        };
        if req.verdict != Verdict::Pending {
            self.store(&file);
            return false;
        }
        req.verdict = if granted {
            Verdict::Granted
        } else {
            Verdict::Denied
        };
        req.decided_at = Some(now);
        self.store(&file);
        true
    }

    /// Take the answer for a capability, consuming it.
    ///
    /// `Some(true)` means proceed, once. `Some(false)` means refused.
    /// `None` means nobody has answered yet.
    pub fn take(&self, capability: &str) -> Option<bool> {
        let mut file = self.load();
        self.sweep(&mut file);

        let idx = file
            .requests
            .iter()
            .position(|r| r.capability == capability && r.verdict != Verdict::Pending)?;
        let decided = file.requests.remove(idx);
        self.store(&file);
        Some(decided.verdict == Verdict::Granted)
    }

    /// Forget everything. For a person who wants to start clean.
    pub fn clear(&self) {
        self.store(&File::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::id::SeqEntropy;

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "km-approve-{}-{}-{}",
                std::process::id(),
                n,
                tag
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        fn file(&self) -> PathBuf {
            self.0.join("approvals.json")
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_request_waits_until_someone_answers() {
        let d = Dir::new("wait");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);

        let id = a.request("stripe.refund", "amount > 100000", "amount=500000");
        assert_eq!(a.pending().len(), 1);
        assert_eq!(a.take("stripe.refund"), None, "nobody has answered yet");

        assert!(a.decide(&id, true));
        assert!(a.pending().is_empty());
        assert_eq!(a.take("stripe.refund"), Some(true));
    }

    #[test]
    fn one_approval_authorises_one_call() {
        let d = Dir::new("once");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);

        let id = a.request("stripe.refund", "rule", "");
        a.decide(&id, true);

        assert_eq!(a.take("stripe.refund"), Some(true));
        assert_eq!(
            a.take("stripe.refund"),
            None,
            "an approval must not become a standing permission"
        );
    }

    #[test]
    fn a_refusal_is_carried_through_rather_than_looking_like_silence() {
        let d = Dir::new("denied");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);

        let id = a.request("stripe.payout_create", "always", "");
        a.decide(&id, false);
        assert_eq!(a.take("stripe.payout_create"), Some(false));
    }

    #[test]
    fn a_request_expires_unanswered() {
        let d = Dir::new("expire");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e).with_ttl(60);

        a.request("stripe.refund", "rule", "");
        c.advance(61);
        assert!(a.pending().is_empty(), "an unanswered request should lapse");
    }

    #[test]
    fn an_approval_granted_long_ago_cannot_be_used_later() {
        let d = Dir::new("stale");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e).with_ttl(60);

        let id = a.request("stripe.refund", "rule", "");
        a.decide(&id, true);
        c.advance(61);

        assert_eq!(
            a.take("stripe.refund"),
            None,
            "a decision must not outlive the request it answered"
        );
    }

    #[test]
    fn retrying_does_not_stack_requests() {
        let d = Dir::new("retry");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);

        let first = a.request("stripe.refund", "rule", "");
        let second = a.request("stripe.refund", "rule", "");
        assert_eq!(first, second, "a retry reuses the waiting request");
        assert_eq!(a.pending().len(), 1);
    }

    #[test]
    fn answering_the_same_request_twice_is_refused() {
        let d = Dir::new("twice");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);

        let id = a.request("stripe.refund", "rule", "");
        assert!(a.decide(&id, true));
        assert!(
            !a.decide(&id, false),
            "a decision cannot be revised in place"
        );
        assert_eq!(a.take("stripe.refund"), Some(true));
    }

    #[test]
    fn answering_something_that_does_not_exist_is_refused() {
        let d = Dir::new("ghost");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);
        assert!(!a.decide("nope", true));
    }

    #[test]
    fn different_capabilities_are_answered_independently() {
        let d = Dir::new("many");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);

        let refund = a.request("stripe.refund", "rule", "");
        a.request("stripe.payout_create", "always", "");
        a.decide(&refund, true);

        assert_eq!(a.take("stripe.refund"), Some(true));
        assert_eq!(a.take("stripe.payout_create"), None, "still waiting");
        assert_eq!(a.pending().len(), 1);
    }

    #[test]
    fn the_queue_survives_a_restart_of_either_side() {
        let d = Dir::new("persist");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();

        let id = {
            let broker = Approvals::new(d.file(), &c, &e);
            broker.request("stripe.refund", "rule", "amount=500000")
        };
        {
            // A different process entirely — the GUI.
            let gui = Approvals::new(d.file(), &c, &e);
            let waiting = gui.pending();
            assert_eq!(waiting.len(), 1);
            assert_eq!(waiting[0].detail, "amount=500000");
            assert!(gui.decide(&id, true));
        }
        let broker = Approvals::new(d.file(), &c, &e);
        assert_eq!(broker.take("stripe.refund"), Some(true));
    }

    #[test]
    fn the_queue_holds_no_values() {
        let d = Dir::new("novalues");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);
        a.request("stripe.refund", "amount > 100000", "amount=500000");

        let text = std::fs::read_to_string(d.file()).unwrap();
        for forbidden in ["sk_live", "secret", "password", "token"] {
            assert!(
                !text.contains(forbidden),
                "the approval queue must never carry a credential: {}",
                text
            );
        }
    }

    #[test]
    fn a_corrupt_file_is_treated_as_empty_rather_than_fatal() {
        let d = Dir::new("corrupt");
        std::fs::write(d.file(), "this is not json").unwrap();
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);

        assert!(a.pending().is_empty());
        // And it recovers: a new request writes a valid file.
        let id = a.request("x", "y", "");
        assert!(a.decide(&id, true));
    }

    #[test]
    fn clearing_removes_everything() {
        let d = Dir::new("clear");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let a = Approvals::new(d.file(), &c, &e);
        a.request("a", "r", "");
        a.request("b", "r", "");
        assert_eq!(a.pending().len(), 2);
        a.clear();
        assert!(a.all().is_empty());
    }
}
