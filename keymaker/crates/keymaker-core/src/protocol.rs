//! The wire format between the thin client and the broker.
//!
//! One JSON object per line, in both directions. The shape enforces the whole
//! point of the project: **no `Response` variant can carry a secret value.**
//! There is no `Read`, no `Get`, no `Reveal`. A tool that returns a value is
//! the one thing that cannot be added later without giving up the claim.

use crate::provider::RequestDraft;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// What the agent may ask for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Open a session. Must come first.
    Hello { version: u32 },
    /// Names of the tasks in the manifest.
    ListTasks,
    /// Names of the endpoint definitions.
    ListEndpoints,
    /// References an environment needs. Names, never values.
    ListRefs { env: String },
    /// Ask for a handle authorising one capability.
    Grant { capability: String, kind: GrantKind },
    /// Spend a handle on a task.
    RunTask { handle: String, env: String },
    /// Spend a handle on one constrained request.
    Call { handle: String, draft: RequestDraft },
    /// Move to the next tool-call, invalidating outstanding handles.
    NextEpoch,
    /// Record a human's answer to a step-up prompt.
    Approve { capability: String, granted: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    Task,
    Request,
}

/// What the broker may answer. Deliberately value-free.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Hello {
        version: u32,
        session: String,
    },
    /// Any listing. Names only.
    Names {
        names: Vec<String>,
    },
    Granted {
        handle: String,
        expires_at: u64,
        uses: u32,
    },
    /// A task's result. Output has already been redacted.
    Ran {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        /// True when a value appeared in output and was masked.
        redacted: bool,
    },
    /// A request's result. The response body has been redacted.
    Called {
        status: u16,
        body: String,
        redacted: bool,
    },
    /// A human is required before this can proceed.
    ApprovalRequired {
        capability: String,
        rule: String,
    },
    Ok,
    Error {
        kind: String,
        message: String,
    },
}

impl Response {
    pub fn error(kind: &str, message: impl std::fmt::Display) -> Response {
        Response::Error {
            kind: kind.into(),
            message: message.to_string(),
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Response::Error { .. })
    }
}

impl From<&crate::error::Error> for Response {
    fn from(e: &crate::error::Error) -> Self {
        use crate::error::Error as E;
        let kind = match e {
            E::Handle(_) => "handle",
            E::Constraint(_) => "constraint",
            E::Denied(_) => "denied",
            E::StepUpRequired(_) => "approval_required",
            E::NotFound(_) => "not_found",
            E::Parse(_) => "parse",
            E::Store(_) => "store",
            E::Os(_) => "os",
        };
        Response::error(kind, e)
    }
}

/// Encode one message as a line. Newlines inside strings are escaped by JSON,
/// so a line is always exactly one message.
pub fn encode<T: Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).unwrap_or_else(|e| {
        serde_json::to_string(&Response::error("encode", e)).expect("error response encodes")
    })
}

pub fn decode_request(line: &str) -> Result<Request, String> {
    serde_json::from_str(line).map_err(|e| e.to_string())
}

pub fn decode_response(line: &str) -> Result<Response, String> {
    serde_json::from_str(line).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let cases = [
            Request::Hello {
                version: PROTOCOL_VERSION,
            },
            Request::ListTasks,
            Request::ListRefs {
                env: "production".into(),
            },
            Request::Grant {
                capability: "stripe.refund".into(),
                kind: GrantKind::Request,
            },
            Request::RunTask {
                handle: "abc".into(),
                env: "production".into(),
            },
            Request::NextEpoch,
            Request::Approve {
                capability: "stripe.refund".into(),
                granted: true,
            },
        ];
        for r in cases {
            assert_eq!(decode_request(&encode(&r)).unwrap(), r);
        }
    }

    #[test]
    fn responses_round_trip() {
        let cases = [
            Response::Hello {
                version: 1,
                session: "s".into(),
            },
            Response::Names {
                names: vec!["deploy".into()],
            },
            Response::Granted {
                handle: "h".into(),
                expires_at: 100,
                uses: 1,
            },
            Response::Ran {
                exit_code: Some(0),
                stdout: "ok".into(),
                stderr: String::new(),
                redacted: false,
            },
            Response::Ok,
            Response::error("denied", "nope"),
        ];
        for r in cases {
            assert_eq!(decode_response(&encode(&r)).unwrap(), r);
        }
    }

    #[test]
    fn a_message_is_always_exactly_one_line() {
        let r = Response::Ran {
            exit_code: Some(1),
            stdout: "line one\nline two\n".into(),
            stderr: "err\nmore\n".into(),
            redacted: true,
        };
        let encoded = encode(&r);
        assert_eq!(
            encoded.lines().count(),
            1,
            "embedded newlines must be escaped"
        );
        assert_eq!(decode_response(&encoded).unwrap(), r);
    }

    #[test]
    fn there_is_no_request_that_asks_for_a_value() {
        // Enumerated deliberately: adding a `Read`/`Get`/`Reveal` op breaks
        // this test, which is the point of it existing.
        let ops = [
            "hello",
            "list_tasks",
            "list_endpoints",
            "list_refs",
            "grant",
            "run_task",
            "call",
            "next_epoch",
            "approve",
        ];
        for forbidden in ["read", "get", "reveal", "export", "fetch_secret", "show"] {
            assert!(
                !ops.contains(&forbidden),
                "`{}` must not be an operation",
                forbidden
            );
        }
        // And the decoder rejects one.
        assert!(decode_request(r#"{"op":"get","key":"stripe/sk_live"}"#).is_err());
        assert!(decode_request(r#"{"op":"read","key":"stripe/sk_live"}"#).is_err());
    }

    #[test]
    fn no_response_variant_has_a_field_that_could_hold_a_value() {
        // Serialise every variant and check the key names. A field called
        // `value`, `secret` or `token` would be the crack in the wall.
        let all = [
            Response::Hello {
                version: 1,
                session: "s".into(),
            },
            Response::Names { names: vec![] },
            Response::Granted {
                handle: "h".into(),
                expires_at: 0,
                uses: 1,
            },
            Response::Ran {
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                redacted: false,
            },
            Response::Called {
                status: 200,
                body: String::new(),
                redacted: false,
            },
            Response::ApprovalRequired {
                capability: "c".into(),
                rule: "r".into(),
            },
            Response::Ok,
            Response::error("k", "m"),
        ];
        for r in all {
            let json: serde_json::Value = serde_json::from_str(&encode(&r)).unwrap();
            let obj = json.as_object().unwrap();
            for forbidden in ["value", "secret", "password", "credential", "token"] {
                assert!(
                    !obj.contains_key(forbidden),
                    "response {:?} exposes a `{}` field",
                    r,
                    forbidden
                );
            }
        }
    }

    #[test]
    fn errors_map_to_stable_kinds() {
        use crate::error::{Error, HandleError};
        let cases = [
            (Error::Handle(HandleError::Expired), "handle"),
            (Error::Constraint("x".into()), "constraint"),
            (Error::Denied("x".into()), "denied"),
            (Error::StepUpRequired("x".into()), "approval_required"),
            (Error::NotFound("x".into()), "not_found"),
        ];
        for (err, expected) in cases {
            match Response::from(&err) {
                Response::Error { kind, .. } => assert_eq!(kind, expected),
                other => panic!("expected an error, got {:?}", other),
            }
        }
    }

    #[test]
    fn a_malformed_line_is_an_error_not_a_panic() {
        assert!(decode_request("not json").is_err());
        assert!(decode_request("").is_err());
        assert!(decode_request(r#"{"op":"unknown_thing"}"#).is_err());
    }
}
