//! Tamper-evident audit log.
//!
//! Every entry carries the hash of the one before it, so removing or editing
//! an entry breaks the chain and `verify` says where. Entries record what was
//! asked for and what was decided — never a value.

use crate::clock::Clock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    SessionOpened { session: String, peer_uid: u32, peer_pid: i32 },
    SessionClosed { session: String },
    HandleIssued { session: String, handle: String, capability: String },
    HandleRedeemed { session: String, handle: String, capability: String },
    HandleRejected { session: String, handle: String, reason: String },
    PolicyDecision { capability: String, decision: String, rule: String },
    RequestSent { capability: String, url: String, status: Option<u16> },
    TaskRun { task: String, command: String, exit_code: Option<i32> },
    LeakDetected { where_: String },
    Approval { capability: String, granted: bool },
}

impl Event {
    fn summary(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "unserialisable".into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub at: u64,
    pub event: Event,
    /// Hash of the previous entry, or 64 zeroes for the first.
    pub prev: String,
    /// Hash over `seq`, `at`, `prev` and the serialised event.
    pub hash: String,
}

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn digest(seq: u64, at: u64, prev: &str, event: &Event) -> String {
    let mut h = Sha256::new();
    h.update(seq.to_be_bytes());
    h.update(at.to_be_bytes());
    h.update(prev.as_bytes());
    h.update(event.summary().as_bytes());
    let out = h.finalize();
    out.iter().map(|b| format!("{:02x}", b)).collect()
}

#[derive(Debug)]
pub struct Log<'a> {
    clock: &'a dyn Clock,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tamper {
    /// The entry at this index does not hash to what it claims.
    Rewritten(usize),
    /// The entry at this index does not follow the one before it.
    Broken(usize),
}

impl<'a> Log<'a> {
    pub fn new(clock: &'a dyn Clock) -> Self {
        Log { clock, entries: Vec::new() }
    }

    pub fn append(&mut self, event: Event) -> &Entry {
        let seq = self.entries.len() as u64;
        let at = self.clock.now();
        let prev = self
            .entries
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_else(|| GENESIS.to_string());
        let hash = digest(seq, at, &prev, &event);
        self.entries.push(Entry { seq, at, event, prev, hash });
        self.entries.last().expect("just pushed")
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn head(&self) -> String {
        self.entries.last().map(|e| e.hash.clone()).unwrap_or_else(|| GENESIS.to_string())
    }

    /// `Ok(())` if the chain is intact, otherwise the first problem found.
    pub fn verify(&self) -> Result<(), Tamper> {
        verify_slice(&self.entries)
    }

    pub fn to_jsonl(&self) -> String {
        self.entries
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn from_jsonl(clock: &'a dyn Clock, src: &str) -> Result<Log<'a>, String> {
        let mut entries = Vec::new();
        for (i, line) in src.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let e: Entry = serde_json::from_str(line)
                .map_err(|e| format!("line {}: {}", i + 1, e))?;
            entries.push(e);
        }
        Ok(Log { clock, entries })
    }
}

fn verify_slice(entries: &[Entry]) -> Result<(), Tamper> {
    let mut prev = GENESIS.to_string();
    for (i, e) in entries.iter().enumerate() {
        if e.prev != prev || e.seq != i as u64 {
            return Err(Tamper::Broken(i));
        }
        if digest(e.seq, e.at, &e.prev, &e.event) != e.hash {
            return Err(Tamper::Rewritten(i));
        }
        prev = e.hash.clone();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;

    fn ev(n: &str) -> Event {
        Event::HandleIssued {
            session: "s".into(),
            handle: n.into(),
            capability: "stripe.refund".into(),
        }
    }

    #[test]
    fn an_empty_log_verifies_and_heads_at_genesis() {
        let c = FixedClock::new(10);
        let log = Log::new(&c);
        assert!(log.is_empty());
        assert_eq!(log.head(), GENESIS);
        assert!(log.verify().is_ok());
    }

    #[test]
    fn entries_chain_to_their_predecessor() {
        let c = FixedClock::new(10);
        let mut log = Log::new(&c);
        log.append(ev("a"));
        c.advance(1);
        log.append(ev("b"));

        assert_eq!(log.len(), 2);
        assert_eq!(log.entries()[0].prev, GENESIS);
        assert_eq!(log.entries()[1].prev, log.entries()[0].hash);
        assert_eq!(log.head(), log.entries()[1].hash);
        assert!(log.verify().is_ok());
    }

    #[test]
    fn editing_an_entry_is_detected() {
        let c = FixedClock::new(10);
        let mut log = Log::new(&c);
        log.append(ev("a"));
        log.append(ev("b"));

        let mut tampered = log.entries().to_vec();
        tampered[0].event = ev("rewritten");
        assert_eq!(verify_slice(&tampered), Err(Tamper::Rewritten(0)));
    }

    #[test]
    fn removing_an_entry_is_detected() {
        let c = FixedClock::new(10);
        let mut log = Log::new(&c);
        log.append(ev("a"));
        log.append(ev("b"));
        log.append(ev("c"));

        let mut tampered = log.entries().to_vec();
        tampered.remove(1);
        assert_eq!(verify_slice(&tampered), Err(Tamper::Broken(1)));
    }

    #[test]
    fn changing_a_timestamp_is_detected() {
        let c = FixedClock::new(10);
        let mut log = Log::new(&c);
        log.append(ev("a"));
        let mut tampered = log.entries().to_vec();
        tampered[0].at += 1;
        assert_eq!(verify_slice(&tampered), Err(Tamper::Rewritten(0)));
    }

    #[test]
    fn a_log_survives_a_round_trip_through_jsonl() {
        let c = FixedClock::new(10);
        let mut log = Log::new(&c);
        log.append(ev("a"));
        c.advance(5);
        log.append(Event::PolicyDecision {
            capability: "stripe.refund".into(),
            decision: "step_up".into(),
            rule: "amount > 100000".into(),
        });

        let text = log.to_jsonl();
        let c2 = FixedClock::new(0);
        let reloaded = Log::from_jsonl(&c2, &text).unwrap();
        assert_eq!(reloaded.entries(), log.entries());
        assert!(reloaded.verify().is_ok());
    }

    #[test]
    fn no_event_variant_carries_a_value_field() {
        // Guards the invariant by inspection of what is serialised.
        let events = [
            ev("a"),
            Event::RequestSent {
                capability: "stripe.refund".into(),
                url: "https://api.stripe.com/v1/refunds".into(),
                status: Some(200),
            },
            Event::TaskRun {
                task: "deploy".into(),
                command: "./deploy.sh".into(),
                exit_code: Some(0),
            },
        ];
        for e in events {
            let json = serde_json::to_string(&e).unwrap();
            for forbidden in ["secret", "value", "password", "token\":"] {
                assert!(
                    !json.contains(forbidden),
                    "audit event must not carry `{}`: {}",
                    forbidden,
                    json
                );
            }
        }
    }
}
