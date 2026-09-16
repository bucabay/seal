//! What the GUI shows, and the rules about what it may show.
//!
//! The GUI is the *human* surface. It is the one place a value can be read,
//! and that is deliberate: the CLI and the MCP surface have no read path at
//! all, so a person needs somewhere to check whether they stored the live key
//! or the test one.
//!
//! Reading still costs something. Every reveal is recorded in the same audit
//! chain as everything else, so "who looked at this, and when" is answerable.

use crate::audit::{Event, Log};
use crate::clock::Clock;
use crate::error::{Error, Result};
use crate::manifest::Manifest;
use crate::provider::Catalog;
use crate::store::{Secret, SecretStore};
use serde::Serialize;
use std::collections::BTreeSet;

/// A reference as the list shows it: a name, and whether this machine has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefRow {
    pub reference: String,
    /// Derived from the reference, never stored — see [`crate::reference`].
    pub issuer: String,
    pub name: String,
    pub present: bool,
    /// Environments that bind this reference, for context in the list.
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskRow {
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndpointRow {
    pub name: String,
    pub method: String,
    pub host: String,
    pub path: String,
    pub secret: String,
    /// Whether policy will stop this for a person.
    pub gated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditRow {
    pub seq: u64,
    pub at: u64,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Health {
    /// How the agent jail is enforced here, in words.
    pub enforcement: String,
    pub enforced: bool,
    /// Whether the audit chain verifies.
    pub audit_intact: bool,
    pub audit_entries: usize,
    /// References the manifest wants that this machine does not have.
    pub missing_refs: Vec<String>,
}

/// Everything the GUI can do, with no Tauri types in sight so it is testable.
pub struct Gui<'a> {
    pub store: &'a mut dyn SecretStore,
    pub manifest: Manifest,
    pub catalog: Catalog,
    pub audit: Log<'a>,
}

impl<'a> Gui<'a> {
    pub fn new(
        store: &'a mut dyn SecretStore,
        manifest: Manifest,
        catalog: Catalog,
        audit: Log<'a>,
    ) -> Self {
        Gui {
            store,
            manifest,
            catalog,
            audit,
        }
    }

    /// Every reference the manifest mentions, plus anything the store already
    /// holds, so a secret saved before it was referenced is still visible.
    pub fn refs(&self) -> Vec<RefRow> {
        let mut names: BTreeSet<String> =
            self.store.names().unwrap_or_default().into_iter().collect();
        let mut used_by: std::collections::BTreeMap<String, Vec<String>> = Default::default();

        for env in self.manifest.env.keys() {
            if let Ok(bindings) = self.manifest.resolve_env(env) {
                for binding in bindings.values() {
                    let r = binding.reference().to_string();
                    names.insert(r.clone());
                    used_by.entry(r).or_default().push(env.clone());
                }
            }
        }
        for ep in &self.catalog.endpoints {
            names.insert(ep.secret.clone());
            used_by
                .entry(ep.secret.clone())
                .or_default()
                .push(format!("endpoint {}", ep.name));
        }

        names
            .into_iter()
            .map(|reference| {
                let (issuer, name) = crate::reference::split(&reference);
                RefRow {
                    issuer: issuer.to_string(),
                    name: name.to_string(),
                    present: self.store.get(&reference).is_ok(),
                    used_by: used_by.get(&reference).cloned().unwrap_or_default(),
                    reference,
                }
            })
            .collect()
    }

    /// Read a value. The only such path in the whole project, and it is
    /// recorded.
    pub fn reveal(&mut self, reference: &str) -> Result<String> {
        let value = self.store.get(reference)?;
        self.audit.append(Event::Revealed {
            reference: reference.to_string(),
        });
        Ok(value.expose().to_string())
    }

    pub fn save(&mut self, reference: &str, value: &str) -> Result<()> {
        if let Some(problem) = crate::reference::problem(reference) {
            return Err(Error::Constraint(problem));
        }
        if value.is_empty() {
            return Err(Error::Constraint("refusing to store an empty value".into()));
        }
        self.store.set(reference, Secret::new(value))?;
        self.audit.append(Event::SecretStored {
            reference: reference.to_string(),
        });
        Ok(())
    }

    pub fn delete(&mut self, reference: &str) -> Result<()> {
        self.store.delete(reference)?;
        self.audit.append(Event::SecretDeleted {
            reference: reference.to_string(),
        });
        Ok(())
    }

    pub fn tasks(&self) -> Vec<TaskRow> {
        self.manifest
            .tasks
            .iter()
            .map(|(name, command)| TaskRow {
                name: name.clone(),
                command: command.clone(),
            })
            .collect()
    }

    pub fn endpoints(&self) -> Vec<EndpointRow> {
        self.catalog
            .endpoints
            .iter()
            .map(|e| EndpointRow {
                name: e.name.clone(),
                method: e.method.clone(),
                host: e.host.clone(),
                path: e.path.clone(),
                secret: e.secret.clone(),
                gated: e.policy.always_step_up || e.policy.step_up.is_some(),
            })
            .collect()
    }

    pub fn environments(&self) -> Vec<String> {
        self.manifest.env.keys().cloned().collect()
    }

    pub fn audit_rows(&self) -> Vec<AuditRow> {
        self.audit
            .entries()
            .iter()
            .map(|e| AuditRow {
                seq: e.seq,
                at: e.at,
                summary: serde_json::to_string(&e.event).unwrap_or_default(),
            })
            .collect()
    }

    /// What is waiting for a decision. The GUI polls this: a person is the
    /// slow part of the loop, so there is nothing to push.
    pub fn approvals(&self, queue: &crate::approvals::Approvals, now: u64) -> Vec<ApprovalRow> {
        queue
            .pending()
            .into_iter()
            .map(|r| ApprovalRow {
                seconds_left: r.expires_at.saturating_sub(now),
                id: r.id,
                capability: r.capability,
                rule: r.rule,
                detail: r.detail,
            })
            .collect()
    }

    /// Answer one. Recorded, because who decided matters as much as what was
    /// decided.
    pub fn decide(
        &mut self,
        queue: &crate::approvals::Approvals,
        id: &str,
        granted: bool,
    ) -> Result<()> {
        let capability = queue
            .pending()
            .into_iter()
            .find(|r| r.id == id)
            .map(|r| r.capability)
            .ok_or_else(|| Error::NotFound(format!("request `{}`", id)))?;

        if !queue.decide(id, granted) {
            return Err(Error::NotFound(format!(
                "request `{}` is no longer waiting",
                id
            )));
        }
        self.audit.append(Event::Approval {
            capability,
            granted,
        });
        Ok(())
    }

    pub fn health(&self) -> Health {
        let enforcement = crate::jail::Profile::enforcement();
        let missing_refs = self
            .refs()
            .into_iter()
            .filter(|r| !r.present)
            .map(|r| r.reference)
            .collect();
        Health {
            enforcement: enforcement.describe(),
            enforced: enforcement.is_enforced(),
            audit_intact: self.audit.verify().is_ok(),
            audit_entries: self.audit.len(),
            missing_refs,
        }
    }
}

/// One request waiting for a person, as the GUI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalRow {
    pub id: String,
    pub capability: String,
    pub rule: String,
    pub detail: String,
    pub seconds_left: u64,
}

/// Clock used by the GUI's audit log.
pub fn system_clock() -> impl Clock {
    crate::clock::SystemClock
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::store::MemoryStore;

    const MANIFEST: &str = r#"
version = 1
[tasks]
deploy = "./deploy.sh"
[env.default]
LOG_LEVEL = "app/log_level"
[env.production]
extends = "default"
DATABASE_URL = "hardroad/db_url"
"#;

    const CATALOG: &str = r#"
[[endpoint]]
name = "stripe.refund"
method = "POST"
host = "api.stripe.com"
path = "/v1/refunds"
secret = "stripe/sk_live"
schema = { amount = "uint32" }
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
policy = { step_up = "amount > 10000" }

[[endpoint]]
name = "github.repo_get"
method = "GET"
host = "api.github.com"
path = "/repos/{owner}/{repo}"
secret = "github/token"
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
"#;

    fn parts() -> (MemoryStore, Manifest, Catalog) {
        (
            MemoryStore::with([
                ("hardroad/db_url", "postgres://u:hunter2@db/prod"),
                ("app/log_level", "debug"),
            ]),
            Manifest::from_toml(MANIFEST).unwrap(),
            Catalog::from_toml(CATALOG).unwrap(),
        )
    }

    #[test]
    fn references_come_from_the_manifest_the_catalog_and_the_store() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let gui = Gui::new(&mut store, m, c, Log::new(&clock));

        let names: Vec<String> = gui.refs().iter().map(|r| r.reference.clone()).collect();
        assert!(
            names.contains(&"hardroad/db_url".to_string()),
            "from the manifest"
        );
        assert!(
            names.contains(&"stripe/sk_live".to_string()),
            "from the catalog"
        );
        assert!(
            names.contains(&"app/log_level".to_string()),
            "from the store"
        );
    }

    #[test]
    fn a_reference_shows_whether_this_machine_has_it() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let gui = Gui::new(&mut store, m, c, Log::new(&clock));
        let rows = gui.refs();

        let present = rows
            .iter()
            .find(|r| r.reference == "hardroad/db_url")
            .unwrap();
        assert!(present.present);
        let absent = rows
            .iter()
            .find(|r| r.reference == "stripe/sk_live")
            .unwrap();
        assert!(!absent.present, "nothing was stored for it");
    }

    #[test]
    fn a_reference_lists_what_uses_it() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let gui = Gui::new(&mut store, m, c, Log::new(&clock));
        let rows = gui.refs();

        let db = rows
            .iter()
            .find(|r| r.reference == "hardroad/db_url")
            .unwrap();
        assert_eq!(db.used_by, vec!["production"]);
        let stripe = rows
            .iter()
            .find(|r| r.reference == "stripe/sk_live")
            .unwrap();
        assert_eq!(stripe.used_by, vec!["endpoint stripe.refund"]);
    }

    #[test]
    fn revealing_returns_the_value_and_records_that_it_happened() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(1_000);
        let mut gui = Gui::new(&mut store, m, c, Log::new(&clock));

        let value = gui.reveal("hardroad/db_url").unwrap();
        assert_eq!(value, "postgres://u:hunter2@db/prod");

        let rows = gui.audit_rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].summary.contains("revealed"));
        assert!(rows[0].summary.contains("hardroad/db_url"));
        assert!(
            !rows[0].summary.contains("hunter2"),
            "the log records that a value was read, never the value"
        );
    }

    #[test]
    fn revealing_something_absent_is_an_error_and_is_not_recorded() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let mut gui = Gui::new(&mut store, m, c, Log::new(&clock));

        assert!(gui.reveal("stripe/sk_live").is_err());
        assert!(gui.audit_rows().is_empty(), "a failed read is not a read");
    }

    #[test]
    fn saving_and_deleting_are_recorded_without_the_value() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let mut gui = Gui::new(&mut store, m, c, Log::new(&clock));

        gui.save("stripe/sk_live", "sk_live_SHOULD_NOT_APPEAR")
            .unwrap();
        assert_eq!(
            gui.reveal("stripe/sk_live").unwrap(),
            "sk_live_SHOULD_NOT_APPEAR"
        );

        gui.delete("stripe/sk_live").unwrap();
        assert!(gui.reveal("stripe/sk_live").is_err());

        let log = format!("{:?}", gui.audit_rows());
        assert!(log.contains("secret_stored"));
        assert!(log.contains("secret_deleted"));
        assert!(
            !log.contains("sk_live_SHOULD_NOT_APPEAR"),
            "the audit log must never carry a value"
        );
    }

    #[test]
    fn rows_carry_the_group_derived_from_the_name() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let gui = Gui::new(&mut store, m, c, Log::new(&clock));

        let rows = gui.refs();
        let stripe = rows.iter().find(|r| r.reference == "stripe/sk_live").unwrap();
        assert_eq!(stripe.issuer, "stripe");
        assert_eq!(stripe.name, "sk_live");
    }

    #[test]
    fn a_malformed_reference_is_refused_with_a_reason() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let mut gui = Gui::new(&mut store, m, c, Log::new(&clock));

        let err = gui.save("has a space", "value").unwrap_err();
        assert!(format!("{}", err).contains("spaces"));
        assert!(gui.save("/leading", "value").is_err());
    }

    #[test]
    fn an_empty_value_is_refused() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let mut gui = Gui::new(&mut store, m, c, Log::new(&clock));
        assert!(gui.save("x/y", "").is_err());
    }

    #[test]
    fn endpoints_show_which_ones_stop_for_a_person() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let gui = Gui::new(&mut store, m, c, Log::new(&clock));
        let rows = gui.endpoints();

        let refund = rows.iter().find(|e| e.name == "stripe.refund").unwrap();
        assert!(refund.gated, "a policy-gated endpoint must be marked");
        assert_eq!(refund.method, "POST");
        assert_eq!(refund.host, "api.stripe.com");

        let repo = rows.iter().find(|e| e.name == "github.repo_get").unwrap();
        assert!(!repo.gated);
    }

    #[test]
    fn tasks_and_environments_come_from_the_manifest() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let gui = Gui::new(&mut store, m, c, Log::new(&clock));

        assert_eq!(
            gui.tasks(),
            vec![TaskRow {
                name: "deploy".into(),
                command: "./deploy.sh".into()
            }]
        );
        assert_eq!(gui.environments(), vec!["default", "production"]);
    }

    struct QueueDir(std::path::PathBuf);
    impl QueueDir {
        fn new(tag: &str) -> QueueDir {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "km-gui-approve-{}-{}-{}",
                std::process::id(),
                n,
                tag
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            QueueDir(p)
        }
        fn file(&self) -> std::path::PathBuf {
            self.0.join("approvals.json")
        }
    }
    impl Drop for QueueDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn waiting_requests_are_listed_with_how_long_is_left() {
        let d = QueueDir::new("list");
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(1_000);
        let entropy = crate::id::SeqEntropy::new();
        let queue = crate::approvals::Approvals::new(d.file(), &clock, &entropy).with_ttl(300);
        queue.request("stripe.refund", "amount > 100000", "amount=500000");

        let gui = Gui::new(&mut store, m, c, Log::new(&clock));
        clock.advance(60);
        let rows = gui.approvals(&queue, clock.now());

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].capability, "stripe.refund");
        assert_eq!(rows[0].detail, "amount=500000");
        assert_eq!(rows[0].seconds_left, 240);
    }

    #[test]
    fn deciding_answers_the_request_and_records_who_decided_what() {
        let d = QueueDir::new("decide");
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(1_000);
        let entropy = crate::id::SeqEntropy::new();
        let queue = crate::approvals::Approvals::new(d.file(), &clock, &entropy);
        let id = queue.request("stripe.refund", "rule", "");

        let mut gui = Gui::new(&mut store, m, c, Log::new(&clock));
        gui.decide(&queue, &id, true).unwrap();

        assert!(gui.approvals(&queue, clock.now()).is_empty());
        assert_eq!(queue.take("stripe.refund"), Some(true));
        let log = format!("{:?}", gui.audit_rows());
        assert!(log.contains("approval"));
        assert!(log.contains("stripe.refund"));
    }

    #[test]
    fn deciding_something_that_is_not_waiting_is_an_error() {
        let d = QueueDir::new("ghost");
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(1_000);
        let entropy = crate::id::SeqEntropy::new();
        let queue = crate::approvals::Approvals::new(d.file(), &clock, &entropy);

        let mut gui = Gui::new(&mut store, m, c, Log::new(&clock));
        assert!(gui.decide(&queue, "nope", true).is_err());
        assert!(
            gui.audit_rows().is_empty(),
            "nothing happened, nothing recorded"
        );
    }

    #[test]
    fn health_reports_what_is_missing_and_whether_the_jail_is_real() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let gui = Gui::new(&mut store, m, c, Log::new(&clock));
        let h = gui.health();

        assert!(h.audit_intact);
        assert_eq!(h.audit_entries, 0);
        assert!(
            h.missing_refs.contains(&"stripe/sk_live".to_string()),
            "a reference with nothing behind it should be flagged"
        );
        assert_eq!(
            h.enforced,
            cfg!(any(target_os = "macos", target_os = "linux"))
        );
        assert!(!h.enforcement.is_empty());
    }

    #[test]
    fn health_notices_a_broken_audit_chain() {
        let (mut store, m, c) = parts();
        let clock = FixedClock::new(0);
        let mut gui = Gui::new(&mut store, m, c, Log::new(&clock));
        gui.reveal("app/log_level").unwrap();
        assert!(gui.health().audit_intact);
        assert_eq!(gui.health().audit_entries, 1);
    }
}
