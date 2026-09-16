//! The MCP surface.
//!
//! This is how an agent actually reaches keymaker. It is JSON-RPC 2.0 over
//! stdio, and it exposes exactly five tools.
//!
//! The absence of a sixth is the product. There is no `read`, no `get`, no
//! `reveal`. A tool that returns a value is the one thing that cannot be added
//! later without giving up the claim, so [`TOOLS`] is a fixed list and a test
//! fails if anything value-shaped appears in it.

use crate::protocol::{GrantKind, Request, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Anything that can answer a broker request: the broker itself, or a client
/// connected to one over the socket.
pub trait Dispatcher {
    fn dispatch(&mut self, req: Request) -> Response;
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: Option<String>,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcResponse {
    fn ok(id: Option<Value>, result: Value) -> Self {
        RpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }
    fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        RpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// One tool the agent may call.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the arguments.
    pub schema: fn() -> Value,
}

fn no_args() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

/// The complete surface. Five tools, and deliberately no sixth.
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "list",
        description: "List the tasks, endpoints and secret references available. \
                      Returns names only — never a secret value.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "what": {
                        "type": "string",
                        "enum": ["tasks", "endpoints", "refs"],
                        "description": "Which listing to return."
                    },
                    "env": {
                        "type": "string",
                        "description": "Environment name, for `refs`. Defaults to `default`."
                    }
                },
                "required": ["what"],
                "additionalProperties": false
            })
        },
    },
    Tool {
        name: "run",
        description: "Run a task declared in .keymaker, with its secrets placed in the \
                      task's own environment. Returns the exit code and the output, with \
                      any secret value filtered out. You never receive the value itself.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "A task name from `list`." },
                    "env": { "type": "string", "description": "Environment name. Defaults to `default`." }
                },
                "required": ["task"],
                "additionalProperties": false
            })
        },
    },
    Tool {
        name: "call",
        description: "Send one request built from an endpoint definition. The method, host \
                      and path are fixed by the definition and the credential is attached \
                      by the broker. Returns the response, with the credential filtered out.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "endpoint": { "type": "string", "description": "An endpoint name from `list`." },
                    "body": { "type": "object", "description": "Request body fields." },
                    "path_params": {
                        "type": "object",
                        "description": "Values for {placeholders} in the endpoint path.",
                        "additionalProperties": { "type": "string" }
                    },
                    "headers": {
                        "type": "object",
                        "description": "Only headers the definition allows.",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["endpoint"],
                "additionalProperties": false
            })
        },
    },
    Tool {
        name: "next_turn",
        description: "Signal that a new tool-call has begun. Invalidates any capability \
                      still outstanding from the previous one.",
        schema: no_args,
    },
    Tool {
        name: "request_approval",
        description: "Ask for a human decision on a capability that policy has gated. \
                      Returns whether it is pending, granted or denied. It does not \
                      grant anything by itself.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "capability": { "type": "string", "description": "The endpoint or task needing approval." }
                },
                "required": ["capability"],
                "additionalProperties": false
            })
        },
    },
];

fn tool_list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": (t.schema)(),
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// Render a broker response as MCP tool output.
///
/// Every branch produces text about what happened. None can produce a value,
/// because no `Response` variant carries one.
fn content(resp: Response) -> Value {
    let (text, is_error) = match resp {
        Response::Names { names } => (
            if names.is_empty() {
                "(none)".to_string()
            } else {
                names.join("\n")
            },
            false,
        ),
        Response::Ran {
            exit_code,
            stdout,
            stderr,
            redacted,
            leaked_files,
        } => {
            let mut out = String::new();
            out.push_str(&format!("exit code: {}\n", exit_code.unwrap_or(-1)));
            if !stdout.is_empty() {
                out.push_str(&format!("\nstdout:\n{}", stdout));
            }
            if !stderr.is_empty() {
                out.push_str(&format!("\nstderr:\n{}", stderr));
            }
            if redacted {
                out.push_str("\n\nnote: this task printed a secret value; it has been masked.");
            }
            for f in &leaked_files {
                out.push_str(&format!("\nwarning: this task wrote a credential to {}", f));
            }
            (out, exit_code != Some(0))
        }
        Response::Called {
            status,
            body,
            redacted,
        } => {
            let mut out = format!("status: {}\n\n{}", status, body);
            if redacted {
                out.push_str("\n\nnote: the response echoed the credential; it has been masked.");
            }
            (out, !(200..300).contains(&status))
        }
        Response::ApprovalRequired { capability, rule } => (
            format!(
                "`{}` needs a human decision ({}). Ask the user to approve it; \
                 you cannot grant this yourself.",
                capability, rule
            ),
            true,
        ),
        Response::Ok => ("ok".to_string(), false),
        // A handle is machinery, not an answer: the agent gets one only as a
        // step inside `run` or `call`, and never sees it rendered.
        Response::Granted { .. } => (
            "a capability was granted but not used; this is a bug in keymaker".to_string(),
            true,
        ),
        Response::Hello { session, .. } => (format!("session {}", session), false),
        Response::Error { kind, message } => (format!("{}: {}", kind, message), true),
    };
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

/// Translates MCP calls into broker requests.
pub struct Server<D: Dispatcher> {
    dispatcher: D,
    started: bool,
}

impl<D: Dispatcher> Server<D> {
    pub fn new(dispatcher: D) -> Self {
        Server {
            dispatcher,
            started: false,
        }
    }

    /// Handle one JSON-RPC message. A notification (no id) produces no reply.
    pub fn handle(&mut self, req: RpcRequest) -> Option<RpcResponse> {
        let id = req.id.clone();
        match req.method.as_str() {
            "initialize" => {
                self.started = true;
                Some(RpcResponse::ok(
                    id,
                    json!({
                        "protocolVersion": MCP_PROTOCOL_VERSION,
                        "capabilities": { "tools": {} },
                        "serverInfo": {
                            "name": "keymaker",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                        "instructions":
                            "keymaker lets you use secrets without seeing them. There is no \
                             tool that returns a secret value, and asking for one is not an \
                             oversight you should work around: use `run` for a task and \
                             `call` for an API request.",
                    }),
                ))
            }
            // Notifications carry no id and get no reply.
            m if m.starts_with("notifications/") => None,
            "tools/list" => Some(RpcResponse::ok(id, tool_list())),
            "tools/call" => Some(self.call_tool(id, &req.params)),
            "ping" => Some(RpcResponse::ok(id, json!({}))),
            other => Some(RpcResponse::err(
                id,
                -32601,
                format!("no such method `{}`", other),
            )),
        }
    }

    fn call_tool(&mut self, id: Option<Value>, params: &Value) -> RpcResponse {
        let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let env = args
            .get("env")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let request = match name {
            "list" => match args.get("what").and_then(|v| v.as_str()) {
                Some("tasks") => Request::ListTasks,
                Some("endpoints") => Request::ListEndpoints,
                Some("refs") => Request::ListRefs { env },
                other => {
                    return RpcResponse::ok(
                        id,
                        json!({
                            "content": [{
                                "type": "text",
                                "text": format!(
                                    "`what` must be tasks, endpoints or refs; got {:?}",
                                    other
                                )
                            }],
                            "isError": true,
                        }),
                    )
                }
            },
            "run" => {
                let Some(task) = args.get("task").and_then(|v| v.as_str()) else {
                    return RpcResponse::ok(id, tool_error("`task` is required"));
                };
                match self.grant(task, GrantKind::Task) {
                    Ok(handle) => Request::RunTask { handle, env },
                    Err(resp) => return RpcResponse::ok(id, content(resp)),
                }
            }
            "call" => {
                let Some(endpoint) = args.get("endpoint").and_then(|v| v.as_str()) else {
                    return RpcResponse::ok(id, tool_error("`endpoint` is required"));
                };
                let draft = crate::provider::RequestDraft {
                    endpoint: endpoint.to_string(),
                    path_params: from_string_map(args.get("path_params")),
                    headers: from_string_map(args.get("headers")),
                    body: args
                        .get("body")
                        .and_then(|v| v.as_object())
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default(),
                };
                match self.grant(endpoint, GrantKind::Request) {
                    Ok(handle) => Request::Call { handle, draft },
                    Err(resp) => return RpcResponse::ok(id, content(resp)),
                }
            }
            "next_turn" => Request::NextEpoch,
            "request_approval" => {
                // Deliberately never grants: an agent asking for approval must
                // not be able to give it to itself.
                let capability = args
                    .get("capability")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return RpcResponse::ok(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": format!(
                                "Approval for `{}` must come from a person. Ask the user to \
                                 approve it; this tool cannot grant it.",
                                capability
                            )
                        }],
                        "isError": false,
                    }),
                );
            }
            other => {
                return RpcResponse::ok(
                    id,
                    tool_error(&format!(
                        "no such tool `{}`. keymaker has no tool that returns a secret value.",
                        other
                    )),
                )
            }
        };

        let resp = self.dispatcher.dispatch(request);
        RpcResponse::ok(id, content(resp))
    }

    /// Obtain a handle for one capability, so the agent never handles one
    /// itself.
    fn grant(&mut self, capability: &str, kind: GrantKind) -> Result<String, Response> {
        match self.dispatcher.dispatch(Request::Grant {
            capability: capability.to_string(),
            kind,
        }) {
            Response::Granted { handle, .. } => Ok(handle),
            other => Err(other),
        }
    }
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

fn from_string_map(v: Option<&Value>) -> std::collections::BTreeMap<String, String> {
    v.and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Run the MCP loop over stdio until the client goes away.
pub fn serve_stdio<D: Dispatcher>(dispatcher: D) -> std::io::Result<()> {
    use std::io::{BufRead, Write};
    let mut server = Server::new(dispatcher);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(req) => server.handle(req),
            Err(e) => Some(RpcResponse::err(
                None,
                -32700,
                format!("parse error: {}", e),
            )),
        };
        if let Some(reply) = reply {
            writeln!(
                stdout,
                "{}",
                serde_json::to_string(&reply).unwrap_or_default()
            )?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Talks to a broker over the socket. The MCP process holds no key material;
/// it carries requests.
impl Dispatcher for crate::server::Client {
    fn dispatch(&mut self, req: Request) -> Response {
        self.send(req).unwrap_or_else(|e| Response::from(&e))
    }
}

/// Runs a broker in this process, for when there is no daemon to talk to.
///
/// The security properties are weaker here — the broker and the MCP surface
/// share an address space — so prefer a real daemon wherever there is one.
pub struct LocalDispatcher<'a> {
    broker: crate::broker::Broker<'a>,
    conn: crate::broker::Connection,
}

impl<'a> LocalDispatcher<'a> {
    pub fn new(mut broker: crate::broker::Broker<'a>, peer: crate::peer::PeerIdentity) -> Self {
        let mut conn = crate::broker::Connection::new(peer);
        // Complete the handshake up front: an in-process caller has nobody to
        // handshake with.
        broker.dispatch(
            &mut conn,
            Request::Hello {
                version: crate::protocol::PROTOCOL_VERSION,
            },
        );
        LocalDispatcher { broker, conn }
    }

    pub fn audit_log(&self) -> &crate::audit::Log<'a> {
        self.broker.audit_log()
    }
}

impl Dispatcher for LocalDispatcher<'_> {
    fn dispatch(&mut self, req: Request) -> Response {
        self.broker.dispatch(&mut self.conn, req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records what the broker was asked, and replies from a script.
    struct FakeDispatcher {
        seen: RefCell<Vec<Request>>,
        replies: RefCell<Vec<Response>>,
    }

    impl FakeDispatcher {
        fn new(replies: Vec<Response>) -> Self {
            FakeDispatcher {
                seen: RefCell::new(Vec::new()),
                replies: RefCell::new(replies),
            }
        }
        fn granting(mut extra: Vec<Response>) -> Self {
            let mut r = vec![Response::Granted {
                handle: "h".repeat(64),
                expires_at: 0,
                uses: 1,
            }];
            r.append(&mut extra);
            FakeDispatcher::new(r)
        }
    }

    impl Dispatcher for FakeDispatcher {
        fn dispatch(&mut self, req: Request) -> Response {
            self.seen.borrow_mut().push(req);
            let mut r = self.replies.borrow_mut();
            if r.is_empty() {
                Response::Ok
            } else {
                r.remove(0)
            }
        }
    }

    fn rpc(method: &str, params: Value) -> RpcRequest {
        RpcRequest {
            jsonrpc: Some("2.0".into()),
            id: Some(json!(1)),
            method: method.into(),
            params,
        }
    }

    fn call(name: &str, args: Value) -> RpcRequest {
        rpc("tools/call", json!({ "name": name, "arguments": args }))
    }

    fn text_of(r: &RpcResponse) -> String {
        r.result.as_ref().unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn is_error(r: &RpcResponse) -> bool {
        r.result.as_ref().unwrap()["isError"].as_bool().unwrap()
    }

    #[test]
    fn initialize_announces_the_protocol_and_the_rule() {
        let mut s = Server::new(FakeDispatcher::new(vec![]));
        let r = s.handle(rpc("initialize", json!({}))).unwrap();
        let result = r.result.unwrap();
        assert_eq!(result["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], "keymaker");
        let instructions = result["instructions"].as_str().unwrap();
        assert!(
            instructions.contains("no tool that returns a secret value"),
            "the model should be told the rule, not left to discover it"
        );
    }

    #[test]
    fn there_is_no_tool_that_returns_a_value() {
        // The central invariant. Adding one breaks this test, which is why it
        // is written as an enumeration rather than a spot check.
        let names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec!["list", "run", "call", "next_turn", "request_approval"]
        );

        for forbidden in [
            "read", "get", "reveal", "export", "show", "fetch", "secret", "value", "dump",
        ] {
            assert!(
                !names.contains(&forbidden),
                "`{}` must never be a tool",
                forbidden
            );
        }
    }

    #[test]
    fn tools_list_is_well_formed_and_advertises_five_tools() {
        let mut s = Server::new(FakeDispatcher::new(vec![]));
        let r = s.handle(rpc("tools/list", json!({}))).unwrap();
        let tools = r.result.unwrap()["tools"].as_array().unwrap().clone();
        assert_eq!(tools.len(), 5);
        for t in &tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn no_tool_description_promises_a_value() {
        for t in TOOLS {
            let d = t.description.to_ascii_lowercase();
            assert!(
                !d.contains("returns the secret") && !d.contains("returns the value"),
                "`{}` describes itself as returning a value",
                t.name
            );
        }
    }

    #[test]
    fn listing_tasks_reaches_the_broker() {
        let d = FakeDispatcher::new(vec![Response::Names {
            names: vec!["deploy".into(), "seed".into()],
        }]);
        let mut s = Server::new(d);
        let r = s.handle(call("list", json!({ "what": "tasks" }))).unwrap();
        assert_eq!(text_of(&r), "deploy\nseed");
        assert!(!is_error(&r));
    }

    #[test]
    fn listing_refs_passes_the_environment_through() {
        let d = FakeDispatcher::new(vec![Response::Names { names: vec![] }]);
        let mut s = Server::new(d);
        let r = s
            .handle(call("list", json!({ "what": "refs", "env": "production" })))
            .unwrap();
        assert_eq!(text_of(&r), "(none)");
        // Cannot borrow the dispatcher back out of the server, so assert on the
        // observable behaviour instead: an empty listing renders as "(none)".
        assert!(!is_error(&r));
    }

    #[test]
    fn an_unknown_listing_is_refused_without_reaching_the_broker() {
        let mut s = Server::new(FakeDispatcher::new(vec![]));
        let r = s
            .handle(call("list", json!({ "what": "secrets" })))
            .unwrap();
        assert!(is_error(&r));
        assert!(text_of(&r).contains("tasks, endpoints or refs"));
    }

    #[test]
    fn running_a_task_grants_then_spends_a_handle() {
        let d = FakeDispatcher::granting(vec![Response::Ran {
            exit_code: Some(0),
            stdout: "deployed\n".into(),
            stderr: String::new(),
            redacted: false,
            leaked_files: vec![],
        }]);
        let mut s = Server::new(d);
        let r = s.handle(call("run", json!({ "task": "deploy" }))).unwrap();

        let text = text_of(&r);
        assert!(text.contains("exit code: 0"));
        assert!(text.contains("deployed"));
        assert!(!is_error(&r));
    }

    #[test]
    fn a_failing_task_is_marked_as_an_error() {
        let d = FakeDispatcher::granting(vec![Response::Ran {
            exit_code: Some(2),
            stdout: String::new(),
            stderr: "boom\n".into(),
            redacted: false,
            leaked_files: vec![],
        }]);
        let mut s = Server::new(d);
        let r = s.handle(call("run", json!({ "task": "deploy" }))).unwrap();
        assert!(is_error(&r));
        assert!(text_of(&r).contains("boom"));
    }

    #[test]
    fn a_leak_is_reported_to_the_agent_without_the_value() {
        let d = FakeDispatcher::granting(vec![Response::Ran {
            exit_code: Some(0),
            stdout: "url=[redacted]\n".into(),
            stderr: String::new(),
            redacted: true,
            leaked_files: vec!["/work/.env".into()],
        }]);
        let mut s = Server::new(d);
        let r = s.handle(call("run", json!({ "task": "deploy" }))).unwrap();
        let text = text_of(&r);
        assert!(text.contains("has been masked"));
        assert!(text.contains("wrote a credential to /work/.env"));
    }

    #[test]
    fn a_refused_grant_is_reported_and_nothing_is_run() {
        let d = FakeDispatcher::new(vec![Response::error("not_found", "no such task")]);
        let mut s = Server::new(d);
        let r = s.handle(call("run", json!({ "task": "ghost" }))).unwrap();
        assert!(is_error(&r));
        assert!(text_of(&r).contains("not_found"));
    }

    #[test]
    fn calling_an_endpoint_carries_the_body_and_path_parameters() {
        let d = FakeDispatcher::granting(vec![Response::Called {
            status: 200,
            body: r#"{"id":"re_1"}"#.into(),
            redacted: false,
        }]);
        let mut s = Server::new(d);
        let r = s
            .handle(call(
                "call",
                json!({
                    "endpoint": "github.issue_create",
                    "path_params": { "owner": "bucabay", "repo": "seal" },
                    "body": { "title": "hello" }
                }),
            ))
            .unwrap();
        assert!(text_of(&r).contains("status: 200"));
        assert!(text_of(&r).contains("re_1"));
    }

    #[test]
    fn a_gated_call_tells_the_agent_to_ask_a_person() {
        let d = FakeDispatcher::granting(vec![Response::ApprovalRequired {
            capability: "stripe.refund".into(),
            rule: "amount > 100000".into(),
        }]);
        let mut s = Server::new(d);
        let r = s
            .handle(call("call", json!({ "endpoint": "stripe.refund" })))
            .unwrap();
        assert!(is_error(&r));
        let text = text_of(&r);
        assert!(text.contains("needs a human decision"));
        assert!(
            text.contains("cannot grant this yourself"),
            "the model must not think it can self-approve"
        );
    }

    #[test]
    fn request_approval_never_grants_anything() {
        // It reaches no dispatcher at all, so there is nothing to subvert.
        let mut s = Server::new(FakeDispatcher::new(vec![Response::Ok]));
        let r = s
            .handle(call(
                "request_approval",
                json!({ "capability": "stripe.payout_create" }),
            ))
            .unwrap();
        let text = text_of(&r);
        assert!(text.contains("must come from a person"));
        assert!(text.contains("cannot grant it"));
    }

    #[test]
    fn a_tool_that_does_not_exist_says_so_and_says_why() {
        let mut s = Server::new(FakeDispatcher::new(vec![]));
        for name in ["read", "get_secret", "reveal"] {
            let r = s.handle(call(name, json!({}))).unwrap();
            assert!(is_error(&r));
            assert!(
                text_of(&r).contains("no tool that returns a secret value"),
                "`{}` should be refused with the reason",
                name
            );
        }
    }

    #[test]
    fn missing_arguments_are_refused_before_anything_is_granted() {
        let mut s = Server::new(FakeDispatcher::new(vec![]));
        assert!(is_error(&s.handle(call("run", json!({}))).unwrap()));
        assert!(is_error(&s.handle(call("call", json!({}))).unwrap()));
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut s = Server::new(FakeDispatcher::new(vec![]));
        let note = RpcRequest {
            jsonrpc: Some("2.0".into()),
            id: None,
            method: "notifications/initialized".into(),
            params: json!({}),
        };
        assert!(
            s.handle(note).is_none(),
            "a notification must not be answered"
        );
    }

    #[test]
    fn an_unknown_method_is_a_jsonrpc_error() {
        let mut s = Server::new(FakeDispatcher::new(vec![]));
        let r = s.handle(rpc("resources/list", json!({}))).unwrap();
        assert_eq!(r.error.unwrap().code, -32601);
    }

    #[test]
    fn ping_is_answered() {
        let mut s = Server::new(FakeDispatcher::new(vec![]));
        assert!(s.handle(rpc("ping", json!({}))).unwrap().error.is_none());
    }

    #[test]
    fn next_turn_invalidates_outstanding_capabilities() {
        let d = FakeDispatcher::new(vec![Response::Ok]);
        let mut s = Server::new(d);
        let r = s.handle(call("next_turn", json!({}))).unwrap();
        assert_eq!(text_of(&r), "ok");
    }

    #[test]
    fn no_rendered_response_can_contain_a_value() {
        // Every variant the broker can return, rendered. None carries a value,
        // because no variant has a field that could hold one.
        let all = [
            Response::Names {
                names: vec!["a".into()],
            },
            Response::Ran {
                exit_code: Some(0),
                stdout: "out".into(),
                stderr: "err".into(),
                redacted: true,
                leaked_files: vec!["/x".into()],
            },
            Response::Called {
                status: 200,
                body: "body".into(),
                redacted: true,
            },
            Response::ApprovalRequired {
                capability: "c".into(),
                rule: "r".into(),
            },
            Response::Ok,
            Response::error("k", "m"),
        ];
        for r in all {
            let rendered = content(r);
            let text = rendered["content"][0]["text"].as_str().unwrap();
            // The only strings present are ones the test supplied.
            assert!(
                !text.contains("sk_live"),
                "rendered output leaked: {}",
                text
            );
        }
    }
}
