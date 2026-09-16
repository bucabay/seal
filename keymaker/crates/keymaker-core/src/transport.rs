//! Sending a prepared request.
//!
//! This is the only code in the project that puts a credential onto a network
//! socket, which is why it is small enough to read in one sitting.
//!
//! Two rules it must not break:
//!
//! - **https only.** A prepared request carries a live credential; sending one
//!   in clear text would hand it to anything on the path. The scheme is checked
//!   here as well as being fixed by the endpoint definition, because defence in
//!   depth costs one `if`.
//! - **No redirect following.** A redirect would move the credential to a host
//!   the endpoint definition never pinned, which is precisely the constraint
//!   the definition exists to enforce.

use crate::broker::{HttpResponse, Transport};
use crate::error::{Error, Result};
use crate::provider::PreparedRequest;

/// Refuses to send anything. The default when the `http` feature is off, and
/// useful in tests that must prove nothing left the machine.
#[derive(Debug, Default, Clone, Copy)]
pub struct OfflineTransport;

impl Transport for OfflineTransport {
    fn send(&self, _req: &PreparedRequest) -> Result<HttpResponse> {
        Err(Error::Os(
            "this build cannot send requests (built without the `http` feature)".into(),
        ))
    }
}

/// Checks that apply regardless of which client does the sending.
pub fn check_sendable(req: &PreparedRequest) -> Result<()> {
    if !req.url.starts_with("https://") {
        return Err(Error::Constraint(format!(
            "refusing to send a credential over a non-https url: {}",
            req.url
        )));
    }
    Ok(())
}

#[cfg(feature = "http")]
mod real {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct HttpTransport {
        timeout: std::time::Duration,
        max_body: usize,
    }

    impl Default for HttpTransport {
        fn default() -> Self {
            HttpTransport {
                timeout: std::time::Duration::from_secs(30),
                // Bound what a remote host can make the broker hold in memory,
                // and what it can push into an agent's context.
                max_body: 1024 * 1024,
            }
        }
    }

    impl HttpTransport {
        pub fn new(timeout_secs: u64, max_body: usize) -> Self {
            HttpTransport {
                timeout: std::time::Duration::from_secs(timeout_secs),
                max_body,
            }
        }
    }

    impl Transport for HttpTransport {
        fn send(&self, req: &PreparedRequest) -> Result<HttpResponse> {
            check_sendable(req)?;

            let agent = ureq::AgentBuilder::new()
                .timeout(self.timeout)
                // A redirect would carry the credential to a host the endpoint
                // definition never pinned.
                .redirects(0)
                .build();

            let mut call = agent.request(&req.method, &req.url);
            for (name, value) in &req.headers {
                call = call.set(name, value);
            }

            let result = match &req.body {
                Some(body) => call
                    .set("Content-Type", "application/json")
                    .send_string(body),
                None => call.call(),
            };

            let (status, reader) = match result {
                Ok(resp) => (resp.status(), resp.into_reader()),
                // A non-2xx is an answer, not a failure: the caller needs the
                // status and the body to know what happened.
                Err(ureq::Error::Status(code, resp)) => (code, resp.into_reader()),
                Err(e) => return Err(Error::Os(format!("sending request: {}", e))),
            };

            use std::io::Read;
            let mut body = String::new();
            reader
                .take(self.max_body as u64)
                .read_to_string(&mut body)
                .map_err(|e| Error::Os(format!("reading response: {}", e)))?;

            Ok(HttpResponse { status, body })
        }
    }
}

#[cfg(feature = "http")]
pub use real::HttpTransport;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn req(url: &str) -> PreparedRequest {
        PreparedRequest {
            method: "POST".into(),
            url: url.into(),
            headers: BTreeMap::from([(
                "Authorization".to_string(),
                "Bearer sk_live_X".to_string(),
            )]),
            body: Some("{}".into()),
            content_type: "application/json",
        }
    }

    #[test]
    fn a_credential_is_never_sent_over_plain_http() {
        let err = check_sendable(&req("http://api.stripe.com/v1/refunds")).unwrap_err();
        assert!(format!("{}", err).contains("non-https"));
    }

    #[test]
    fn https_passes_the_check() {
        assert!(check_sendable(&req("https://api.stripe.com/v1/refunds")).is_ok());
    }

    #[test]
    fn other_schemes_are_refused_too() {
        for url in [
            "file:///etc/passwd",
            "ftp://host/x",
            "//api.stripe.com/x",
            "",
        ] {
            assert!(
                check_sendable(&req(url)).is_err(),
                "`{}` must not be sendable",
                url
            );
        }
    }

    #[test]
    fn the_offline_transport_sends_nothing_and_says_so() {
        let err = OfflineTransport
            .send(&req("https://api.stripe.com/v1/refunds"))
            .unwrap_err();
        assert!(format!("{}", err).contains("cannot send"));
    }

    #[cfg(feature = "http")]
    #[test]
    fn the_real_transport_refuses_plain_http_before_opening_a_socket() {
        let err = HttpTransport::default()
            .send(&req("http://127.0.0.1:1/never"))
            .unwrap_err();
        assert!(
            format!("{}", err).contains("non-https"),
            "the scheme must be rejected before any connection is attempted"
        );
    }
}
