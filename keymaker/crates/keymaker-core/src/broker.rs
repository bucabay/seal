//! The broker: the only process that ever holds a plaintext value.
//!
//! The agent connects, is identified by the kernel rather than by anything it
//! says, and from then on can ask for *actions*. Every path through here ends
//! in an effect or an error — never a value.

use crate::audit::{Event, Log};
use crate::clock::Clock;
use crate::error::{Error, Result};
use crate::handle::{Capability, Registry, SessionId, SessionPolicy};
use crate::id::Id;
use crate::manifest::Manifest;
use crate::peer::PeerIdentity;
use crate::policy::Decision;
use crate::protocol::{GrantKind, Request, Response, PROTOCOL_VERSION};
use crate::provider::{Catalog, PreparedRequest};
use crate::redact::Redactor;
use crate::runner::{Runner, Spawner};
use crate::store::SecretStore;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// Sends a prepared request. Separated so the broker is testable without a
/// network, and so the only code that touches a credential-bearing request is
/// one small implementation.
pub trait Transport: std::fmt::Debug {
    fn send(&self, req: &PreparedRequest) -> Result<HttpResponse>;
}

/// Per-connection state. The session lives and dies with the socket.
#[derive(Debug)]
pub struct Connection {
    pub peer: PeerIdentity,
    pub session: Option<SessionId>,
    /// Capabilities a human has approved during this connection.
    approved: BTreeSet<String>,
}

impl Connection {
    pub fn new(peer: PeerIdentity) -> Self {
        Connection {
            peer,
            session: None,
            approved: BTreeSet::new(),
        }
    }
}

pub struct Broker<'a> {
    pub manifest: Manifest,
    pub catalog: Catalog,
    store: &'a dyn SecretStore,
    spawner: &'a dyn Spawner,
    transport: &'a dyn Transport,
    registry: Registry<'a>,
    audit: Log<'a>,
    default_uses: u32,
}

impl std::fmt::Debug for Broker<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Broker")
            .field("tasks", &self.manifest.task_names().len())
            .field("endpoints", &self.catalog.names().len())
            .field("outstanding_handles", &self.registry.outstanding())
            .finish()
    }
}

impl<'a> Broker<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: &'a dyn Clock,
        entropy: &'a dyn crate::id::Entropy,
        store: &'a dyn SecretStore,
        spawner: &'a dyn Spawner,
        transport: &'a dyn Transport,
        manifest: Manifest,
        catalog: Catalog,
        handle_ttl: u64,
    ) -> Self {
        Broker {
            manifest,
            catalog,
            store,
            spawner,
            transport,
            registry: Registry::new(clock, entropy, handle_ttl),
            audit: Log::new(clock),
            default_uses: 1,
        }
    }

    /// Write the audit trail to a file as well as memory. Without this the
    /// log answers nothing after the broker exits, which is when it is usually
    /// wanted.
    pub fn with_audit(mut self, log: Log<'a>) -> Self {
        self.audit = log;
        self
    }

    pub fn audit_log(&self) -> &Log<'a> {
        &self.audit
    }

    /// End a session and destroy every handle it held. Called when the socket
    /// closes, so a handle cannot outlive the connection that earned it.
    pub fn close_session(&mut self, session: &SessionId) {
        self.registry.close_session(session);
        self.audit.append(Event::SessionClosed {
            session: session.to_string(),
        });
    }

    /// Parse a session id received over the wire.
    pub fn session_from_str(s: &str) -> Option<SessionId> {
        Id::parse(s)
    }

    /// Handle one message. Infallible by construction: every error becomes a
    /// `Response::Error`, so a malformed or hostile request can never take the
    /// broker down.
    pub fn dispatch(&mut self, conn: &mut Connection, req: Request) -> Response {
        // Every request but Hello needs a live session, and the peer must
        // still be the process we admitted.
        if !matches!(req, Request::Hello { .. }) {
            if conn.session.is_none() {
                return Response::error("no_session", "send hello first");
            }
            if !conn.peer.still_the_same_process() {
                let session = conn.session.take();
                if let Some(s) = &session {
                    self.registry.close_session(s);
                    self.audit.append(Event::SessionClosed {
                        session: s.to_string(),
                    });
                }
                return Response::error(
                    "peer_changed",
                    "the calling process is no longer the one that connected",
                );
            }
        }

        match req {
            Request::Hello { version } => self.hello(conn, version),
            Request::ListTasks => Response::Names {
                names: self
                    .manifest
                    .task_names()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            },
            Request::ListEndpoints => Response::Names {
                names: self.catalog.names().iter().map(|s| s.to_string()).collect(),
            },
            Request::ListRefs { env } => match self.manifest.refs_for(&env) {
                Ok(names) => Response::Names { names },
                Err(e) => Response::from(&e),
            },
            Request::Grant { capability, kind } => self.grant(conn, &capability, kind),
            Request::NextEpoch => {
                let session = conn.session.clone().expect("checked above");
                match self.registry.advance_epoch(&session) {
                    Ok(_) => Response::Ok,
                    Err(e) => Response::from(&Error::Handle(e)),
                }
            }
            Request::Approve {
                capability,
                granted,
            } => {
                if granted {
                    conn.approved.insert(capability.clone());
                } else {
                    conn.approved.remove(&capability);
                }
                self.audit.append(Event::Approval {
                    capability,
                    granted,
                });
                Response::Ok
            }
            Request::RunTask { handle, env } => self.run_task(conn, &handle, &env),
            Request::Call { handle, draft } => self.call(conn, &handle, draft),
        }
    }

    fn hello(&mut self, conn: &mut Connection, version: u32) -> Response {
        if version != PROTOCOL_VERSION {
            return Response::error(
                "version",
                format!(
                    "broker speaks {}, client speaks {}",
                    PROTOCOL_VERSION, version
                ),
            );
        }
        if !conn.peer.is_same_user_as_us() {
            return Response::error("forbidden", "the broker serves one user only");
        }
        let session = self.registry.open_session(SessionPolicy::default());
        self.audit.append(Event::SessionOpened {
            session: session.to_string(),
            peer_uid: conn.peer.uid,
            peer_pid: conn.peer.pid,
        });
        conn.session = Some(session.clone());
        Response::Hello {
            version: PROTOCOL_VERSION,
            session: session.to_string(),
        }
    }

    fn grant(&mut self, conn: &mut Connection, capability: &str, kind: GrantKind) -> Response {
        let session = conn.session.clone().expect("checked by dispatch");

        // A handle is only issued for something that actually exists, so the
        // grant call cannot be used to probe for names.
        let cap = match kind {
            GrantKind::Task => {
                if self.manifest.task(capability).is_err() {
                    return Response::from(&Error::NotFound(format!("task `{}`", capability)));
                }
                Capability::Task(capability.to_string())
            }
            GrantKind::Request => {
                if self.catalog.get(capability).is_err() {
                    return Response::from(&Error::NotFound(format!("endpoint `{}`", capability)));
                }
                Capability::Request(capability.to_string())
            }
        };

        match self.registry.issue(&session, cap, self.default_uses) {
            Ok(g) => {
                self.audit.append(Event::HandleIssued {
                    session: session.to_string(),
                    handle: g.id.short().to_string(),
                    capability: capability.to_string(),
                });
                Response::Granted {
                    handle: g.id.to_string(),
                    expires_at: g.expires_at,
                    uses: g.uses_left,
                }
            }
            Err(e) => Response::from(&Error::Handle(e)),
        }
    }

    /// Spend a handle, checking that it authorises the thing being attempted.
    fn redeem(
        &mut self,
        conn: &Connection,
        handle: &str,
        want: &str,
        kind: GrantKind,
    ) -> Result<()> {
        let session = conn.session.clone().expect("checked by dispatch");
        let id = Id::parse(handle).ok_or(Error::Handle(crate::error::HandleError::Unknown))?;
        let red = self.registry.redeem(&session, &id).map_err(|e| {
            self.audit.append(Event::HandleRejected {
                session: session.to_string(),
                handle: id.short().to_string(),
                reason: e.to_string(),
            });
            Error::Handle(e)
        })?;

        let matches = match (&red.capability, kind) {
            (Capability::Task(n), GrantKind::Task) => n == want,
            (Capability::Request(n), GrantKind::Request) => n == want,
            _ => false,
        };
        if !matches {
            return Err(Error::Constraint(format!(
                "this handle does not authorise `{}`",
                want
            )));
        }
        self.audit.append(Event::HandleRedeemed {
            session: session.to_string(),
            handle: id.short().to_string(),
            capability: want.to_string(),
        });
        Ok(())
    }

    fn run_task(&mut self, conn: &mut Connection, handle: &str, env: &str) -> Response {
        // The handle names the task, so the agent cannot redeem a handle for
        // one task and run another.
        let session = conn.session.clone().expect("checked by dispatch");
        let id = match Id::parse(handle) {
            Some(i) => i,
            None => return Response::error("handle", "unknown handle"),
        };
        let task = match self.registry.redeem(&session, &id) {
            Ok(r) => match r.capability {
                Capability::Task(name) => name,
                Capability::Request(_) => {
                    return Response::error(
                        "constraint",
                        "this handle is for a request, not a task",
                    )
                }
            },
            Err(e) => {
                self.audit.append(Event::HandleRejected {
                    session: session.to_string(),
                    handle: id.short().to_string(),
                    reason: e.to_string(),
                });
                return Response::from(&Error::Handle(e));
            }
        };

        let command = match self.manifest.task(&task) {
            Ok(c) => c.to_string(),
            Err(e) => return Response::from(&e),
        };
        // The agent is blocked while this runs, so this is the window in which
        // the broker is the only party to have seen what the task wrote.
        let runner = Runner::new(self.store, self.spawner).watching_defaults();
        match runner.run_task(&self.manifest, &task, env) {
            Ok(out) => {
                self.audit.append(Event::TaskRun {
                    task: task.clone(),
                    command,
                    exit_code: out.exit_code,
                });
                if out.redacted {
                    self.audit.append(Event::LeakDetected {
                        where_: format!("task `{}` output", task),
                    });
                }
                for path in &out.files_with_values {
                    self.audit.append(Event::LeakDetected {
                        where_: format!("file written by `{}`: {}", task, path.display()),
                    });
                }
                Response::Ran {
                    exit_code: out.exit_code,
                    stdout: out.stdout_string(),
                    stderr: out.stderr_string(),
                    redacted: out.redacted,
                    leaked_files: out
                        .files_with_values
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect(),
                }
            }
            Err(e) => Response::from(&e),
        }
    }

    fn call(
        &mut self,
        conn: &mut Connection,
        handle: &str,
        draft: crate::provider::RequestDraft,
    ) -> Response {
        let name = draft.endpoint.clone();
        if let Err(e) = self.redeem(conn, handle, &name, GrantKind::Request) {
            return Response::from(&e);
        }
        let endpoint = match self.catalog.get(&name) {
            Ok(e) => e.clone(),
            Err(e) => return Response::from(&e),
        };

        // Structural checks first: a rejected request never reaches the store.
        let checked = match endpoint.check(&draft) {
            Ok(c) => c,
            Err(e) => return Response::from(&e),
        };

        match endpoint.decide(&checked) {
            Decision::Allow => {}
            Decision::Deny(rule) => {
                self.audit.append(Event::PolicyDecision {
                    capability: name.clone(),
                    decision: "deny".into(),
                    rule: rule.clone(),
                });
                return Response::from(&Error::Denied(rule));
            }
            Decision::StepUp(rule) => {
                if !conn.approved.contains(&name) {
                    self.audit.append(Event::PolicyDecision {
                        capability: name.clone(),
                        decision: "step_up".into(),
                        rule: rule.clone(),
                    });
                    return Response::ApprovalRequired {
                        capability: name,
                        rule,
                    };
                }
                // An approval is spent by the call it permitted.
                conn.approved.remove(&name);
            }
        }

        let secret = match self.store.get(&endpoint.secret) {
            Ok(s) => s,
            Err(e) => return Response::from(&e),
        };
        let prepared = endpoint.prepare(checked, secret.expose());
        let url = prepared.url.clone();

        match self.transport.send(&prepared) {
            Ok(resp) => {
                // The response may echo the credential back; filter it.
                let redactor = Redactor::new(&[secret.expose()]);
                let leaked = redactor.detects(resp.body.as_bytes());
                let body =
                    String::from_utf8_lossy(&redactor.redact(resp.body.as_bytes())).into_owned();
                self.audit.append(Event::RequestSent {
                    capability: name.clone(),
                    url,
                    status: Some(resp.status),
                });
                if leaked {
                    self.audit.append(Event::LeakDetected {
                        where_: format!("response body of `{}`", name),
                    });
                }
                Response::Called {
                    status: resp.status,
                    body,
                    redacted: leaked,
                }
            }
            Err(e) => {
                self.audit.append(Event::RequestSent {
                    capability: name,
                    url,
                    status: None,
                });
                Response::from(&e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::id::SeqEntropy;
    use crate::peer::current_uid;
    use crate::provider::RequestDraft;
    use crate::runner::RawOutcome;
    use crate::store::MemoryStore;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    const MANIFEST: &str = r#"
version = 1
[tasks]
deploy = "./deploy.sh"
seed = "./seed.sh"
[env.production]
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
name = "stripe.blocked"
method = "POST"
host = "api.stripe.com"
path = "/v1/payouts"
secret = "stripe/sk_live"
schema = { amount = "uint32" }
inject = { kind = "header", name = "Authorization", format = "Bearer {secret}" }
policy = { deny = "amount > 0" }
"#;

    #[derive(Debug)]
    struct FakeSpawner(RawOutcome);
    impl Spawner for FakeSpawner {
        fn spawn(&self, _: &str, _: &BTreeMap<String, String>) -> Result<RawOutcome> {
            Ok(self.0.clone())
        }
    }

    #[derive(Debug)]
    struct FakeTransport {
        reply: HttpResponse,
        sent: RefCell<Vec<PreparedRequest>>,
    }
    impl FakeTransport {
        fn new(body: &str) -> Self {
            FakeTransport {
                reply: HttpResponse {
                    status: 200,
                    body: body.into(),
                },
                sent: RefCell::new(Vec::new()),
            }
        }
    }
    impl Transport for FakeTransport {
        fn send(&self, req: &PreparedRequest) -> Result<HttpResponse> {
            self.sent.borrow_mut().push(req.clone());
            Ok(self.reply.clone())
        }
    }

    struct Fixture {
        clock: FixedClock,
        entropy: SeqEntropy,
        store: MemoryStore,
        spawner: FakeSpawner,
        transport: FakeTransport,
    }

    impl Fixture {
        fn new() -> Self {
            Fixture {
                clock: FixedClock::new(1_000),
                entropy: SeqEntropy::new(),
                store: MemoryStore::with([
                    ("hardroad/db_url", "postgres://u:hunter2@db/prod"),
                    ("stripe/sk_live", "sk_live_REALVALUE"),
                ]),
                spawner: FakeSpawner(RawOutcome {
                    exit_code: Some(0),
                    stdout: b"done\n".to_vec(),
                    stderr: Vec::new(),
                }),
                transport: FakeTransport::new(r#"{"id":"re_1","status":"succeeded"}"#),
            }
        }

        fn broker(&self) -> Broker<'_> {
            Broker::new(
                &self.clock,
                &self.entropy,
                &self.store,
                &self.spawner,
                &self.transport,
                Manifest::from_toml(MANIFEST).unwrap(),
                Catalog::from_toml(CATALOG).unwrap(),
                60,
            )
        }
    }

    /// This process, which is trivially still itself.
    fn peer() -> PeerIdentity {
        PeerIdentity {
            uid: current_uid(),
            gid: 0,
            pid: std::process::id() as i32,
            start_time: None,
            cwd: None,
        }
    }

    /// Connect and complete the handshake.
    fn connected(b: &mut Broker) -> Connection {
        let mut c = Connection::new(peer());
        let r = b.dispatch(
            &mut c,
            Request::Hello {
                version: PROTOCOL_VERSION,
            },
        );
        assert!(
            matches!(r, Response::Hello { .. }),
            "handshake failed: {:?}",
            r
        );
        c
    }

    fn grant(b: &mut Broker, c: &mut Connection, cap: &str, kind: GrantKind) -> String {
        match b.dispatch(
            c,
            Request::Grant {
                capability: cap.into(),
                kind,
            },
        ) {
            Response::Granted { handle, .. } => handle,
            other => panic!("expected a grant, got {:?}", other),
        }
    }

    #[test]
    fn a_session_must_be_opened_before_anything_else() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = Connection::new(peer());
        let r = b.dispatch(&mut c, Request::ListTasks);
        assert!(matches!(&r, Response::Error { kind, .. } if kind == "no_session"));
    }

    #[test]
    fn a_version_mismatch_is_refused() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = Connection::new(peer());
        let r = b.dispatch(&mut c, Request::Hello { version: 999 });
        assert!(matches!(&r, Response::Error { kind, .. } if kind == "version"));
        assert!(c.session.is_none());
    }

    #[test]
    fn a_connection_from_another_user_is_refused() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut other = Connection::new(PeerIdentity {
            uid: current_uid().wrapping_add(1),
            ..peer()
        });
        let r = b.dispatch(
            &mut other,
            Request::Hello {
                version: PROTOCOL_VERSION,
            },
        );
        assert!(matches!(&r, Response::Error { kind, .. } if kind == "forbidden"));
        assert!(other.session.is_none());
    }

    #[test]
    fn listings_return_names_only() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);

        let Response::Names { names } = b.dispatch(&mut c, Request::ListTasks) else {
            panic!("expected names")
        };
        assert_eq!(names, vec!["deploy", "seed"]);

        let Response::Names { names } = b.dispatch(&mut c, Request::ListEndpoints) else {
            panic!("expected names")
        };
        assert_eq!(names, vec!["stripe.refund", "stripe.blocked"]);

        let Response::Names { names } = b.dispatch(
            &mut c,
            Request::ListRefs {
                env: "production".into(),
            },
        ) else {
            panic!("expected names")
        };
        assert_eq!(names, vec!["hardroad/db_url"]);
        assert!(
            !format!("{:?}", names).contains("hunter2"),
            "a listing must never carry a value"
        );
    }

    #[test]
    fn a_grant_is_refused_for_something_that_does_not_exist() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        for (cap, kind) in [
            ("ghost", GrantKind::Task),
            ("ghost.thing", GrantKind::Request),
        ] {
            let r = b.dispatch(
                &mut c,
                Request::Grant {
                    capability: cap.into(),
                    kind,
                },
            );
            assert!(matches!(&r, Response::Error { kind, .. } if kind == "not_found"));
        }
    }

    #[test]
    fn a_granted_handle_runs_its_task() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "deploy", GrantKind::Task);

        let r = b.dispatch(
            &mut c,
            Request::RunTask {
                handle: h,
                env: "production".into(),
            },
        );
        match r {
            Response::Ran {
                exit_code, stdout, ..
            } => {
                assert_eq!(exit_code, Some(0));
                assert_eq!(stdout, "done\n");
            }
            other => panic!("expected a run, got {:?}", other),
        }
    }

    #[test]
    fn a_handle_cannot_be_spent_twice() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "deploy", GrantKind::Task);

        let first = b.dispatch(
            &mut c,
            Request::RunTask {
                handle: h.clone(),
                env: "production".into(),
            },
        );
        assert!(matches!(first, Response::Ran { .. }));

        let second = b.dispatch(
            &mut c,
            Request::RunTask {
                handle: h,
                env: "production".into(),
            },
        );
        assert!(matches!(&second, Response::Error { kind, .. } if kind == "handle"));
    }

    #[test]
    fn a_handle_for_a_request_cannot_run_a_task() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "stripe.refund", GrantKind::Request);

        let r = b.dispatch(
            &mut c,
            Request::RunTask {
                handle: h,
                env: "production".into(),
            },
        );
        assert!(matches!(&r, Response::Error { kind, .. } if kind == "constraint"));
    }

    #[test]
    fn a_handle_for_one_endpoint_cannot_call_another() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "stripe.refund", GrantKind::Request);

        let draft = RequestDraft {
            endpoint: "stripe.blocked".into(),
            body: [("amount".to_string(), serde_json::json!(1))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let r = b.dispatch(&mut c, Request::Call { handle: h, draft });
        assert!(
            matches!(&r, Response::Error { kind, .. } if kind == "handle" || kind == "constraint")
        );
        assert!(f.transport.sent.borrow().is_empty(), "nothing may be sent");
    }

    #[test]
    fn a_forged_handle_is_rejected() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        for bogus in ["not-a-handle", &"f".repeat(64), ""] {
            let r = b.dispatch(
                &mut c,
                Request::RunTask {
                    handle: bogus.into(),
                    env: "production".into(),
                },
            );
            assert!(matches!(&r, Response::Error { kind, .. } if kind == "handle"));
        }
    }

    #[test]
    fn an_allowed_call_is_sent_with_the_credential_attached() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "stripe.refund", GrantKind::Request);

        let draft = RequestDraft {
            endpoint: "stripe.refund".into(),
            body: [("amount".to_string(), serde_json::json!(500))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let r = b.dispatch(&mut c, Request::Call { handle: h, draft });
        match r {
            Response::Called { status, body, .. } => {
                assert_eq!(status, 200);
                assert!(body.contains("re_1"));
            }
            other => panic!("expected a call, got {:?}", other),
        }

        let sent = f.transport.sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url, "https://api.stripe.com/v1/refunds");
        assert_eq!(
            sent[0].headers.get("Authorization").unwrap(),
            "Bearer sk_live_REALVALUE"
        );
    }

    #[test]
    fn the_response_never_carries_the_credential_back() {
        let f = Fixture {
            transport: FakeTransport::new(r#"{"echo":"sk_live_REALVALUE"}"#),
            ..Fixture::new()
        };
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "stripe.refund", GrantKind::Request);

        let draft = RequestDraft {
            endpoint: "stripe.refund".into(),
            body: [("amount".to_string(), serde_json::json!(1))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        match b.dispatch(&mut c, Request::Call { handle: h, draft }) {
            Response::Called { body, redacted, .. } => {
                assert!(
                    !body.contains("sk_live_REALVALUE"),
                    "credential echoed back: {}",
                    body
                );
                assert!(redacted, "an echoed credential must be reported");
            }
            other => panic!("expected a call, got {:?}", other),
        }
    }

    #[test]
    fn a_denied_call_never_reaches_the_store_or_the_network() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "stripe.blocked", GrantKind::Request);

        let draft = RequestDraft {
            endpoint: "stripe.blocked".into(),
            body: [("amount".to_string(), serde_json::json!(5))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let r = b.dispatch(&mut c, Request::Call { handle: h, draft });
        assert!(matches!(&r, Response::Error { kind, .. } if kind == "denied"));
        assert!(
            f.transport.sent.borrow().is_empty(),
            "a denied call must not be sent"
        );
    }

    #[test]
    fn a_step_up_call_waits_for_a_human_then_proceeds_once() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);

        let big = || RequestDraft {
            endpoint: "stripe.refund".into(),
            body: [("amount".to_string(), serde_json::json!(50_000))]
                .into_iter()
                .collect(),
            ..Default::default()
        };

        // First attempt: blocked pending approval, and nothing was sent.
        let h1 = grant(&mut b, &mut c, "stripe.refund", GrantKind::Request);
        let r = b.dispatch(
            &mut c,
            Request::Call {
                handle: h1,
                draft: big(),
            },
        );
        assert!(matches!(r, Response::ApprovalRequired { .. }));
        assert!(f.transport.sent.borrow().is_empty());

        // A human says yes.
        b.dispatch(
            &mut c,
            Request::Approve {
                capability: "stripe.refund".into(),
                granted: true,
            },
        );

        let h2 = grant(&mut b, &mut c, "stripe.refund", GrantKind::Request);
        let r = b.dispatch(
            &mut c,
            Request::Call {
                handle: h2,
                draft: big(),
            },
        );
        assert!(
            matches!(r, Response::Called { .. }),
            "approved call should proceed"
        );
        assert_eq!(f.transport.sent.borrow().len(), 1);

        // The approval is spent: a second large refund must ask again.
        let h3 = grant(&mut b, &mut c, "stripe.refund", GrantKind::Request);
        let r = b.dispatch(
            &mut c,
            Request::Call {
                handle: h3,
                draft: big(),
            },
        );
        assert!(
            matches!(r, Response::ApprovalRequired { .. }),
            "one approval must authorise exactly one call"
        );
        assert_eq!(f.transport.sent.borrow().len(), 1);
    }

    #[test]
    fn a_new_tool_call_invalidates_outstanding_handles() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "deploy", GrantKind::Task);

        assert!(matches!(
            b.dispatch(&mut c, Request::NextEpoch),
            Response::Ok
        ));

        let r = b.dispatch(
            &mut c,
            Request::RunTask {
                handle: h,
                env: "production".into(),
            },
        );
        assert!(matches!(&r, Response::Error { kind, .. } if kind == "handle"));
    }

    #[test]
    fn a_handle_expires() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "deploy", GrantKind::Task);

        f.clock.advance(61);
        let r = b.dispatch(
            &mut c,
            Request::RunTask {
                handle: h,
                env: "production".into(),
            },
        );
        assert!(matches!(&r, Response::Error { kind, .. } if kind == "handle"));
    }

    #[test]
    fn a_leaking_task_is_reported_and_recorded() {
        let f = Fixture {
            spawner: FakeSpawner(RawOutcome {
                exit_code: Some(0),
                stdout: b"url=postgres://u:hunter2@db/prod\n".to_vec(),
                stderr: Vec::new(),
            }),
            ..Fixture::new()
        };
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "deploy", GrantKind::Task);

        match b.dispatch(
            &mut c,
            Request::RunTask {
                handle: h,
                env: "production".into(),
            },
        ) {
            Response::Ran {
                stdout, redacted, ..
            } => {
                assert!(!stdout.contains("hunter2"));
                assert!(redacted);
            }
            other => panic!("expected a run, got {:?}", other),
        }
        let log = format!("{:?}", b.audit_log().entries());
        assert!(log.contains("LeakDetected"));
        assert!(
            !log.contains("hunter2"),
            "the audit log must not record the value"
        );
    }

    #[test]
    fn the_audit_chain_covers_the_whole_session_and_verifies() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "deploy", GrantKind::Task);
        b.dispatch(
            &mut c,
            Request::RunTask {
                handle: h,
                env: "production".into(),
            },
        );

        assert!(b.audit_log().verify().is_ok());
        let kinds: Vec<&str> = b
            .audit_log()
            .entries()
            .iter()
            .map(|e| match e.event {
                Event::SessionOpened { .. } => "open",
                Event::HandleIssued { .. } => "issued",
                Event::HandleRedeemed { .. } => "redeemed",
                Event::TaskRun { .. } => "ran",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["open", "issued", "ran"]);
    }

    #[test]
    fn a_full_handle_is_never_written_to_the_audit_log() {
        let f = Fixture::new();
        let mut b = f.broker();
        let mut c = connected(&mut b);
        let h = grant(&mut b, &mut c, "deploy", GrantKind::Task);

        let log = format!("{:?}", b.audit_log().entries());
        assert!(
            !log.contains(&h),
            "the log must record a prefix, not a redeemable handle"
        );
        assert!(log.contains(&h[..8]));
    }
}
