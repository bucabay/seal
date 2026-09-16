//! AWS Signature Version 4.
//!
//! Needed because the most useful ephemeral credential on AWS — an STS
//! `AssumeRole` result — can only be obtained by making a signed request with
//! the long-lived key. The broker does that so nothing else has to hold the key.
//!
//! The algorithm is fiddly and every step is exact, so this is checked against
//! AWS's published `get-vanilla` test vector rather than only against itself.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type HmacSha256 = Hmac<Sha256>;

pub const ALGORITHM: &str = "AWS4-HMAC-SHA256";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

/// Percent-encoding for SigV4. Unlike ordinary URL encoding, `/` is escaped in
/// query strings but not in the path, so the two cases are separated.
fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

/// Everything the signature covers.
#[derive(Debug, Clone)]
pub struct CanonicalRequest {
    pub method: String,
    pub path: String,
    /// Query parameters, sorted and encoded by `canonical`.
    pub query: Vec<(String, String)>,
    /// Headers to sign. `host` is required; `x-amz-date` is added by `sign`.
    pub headers: BTreeMap<String, String>,
    pub payload: Vec<u8>,
}

impl CanonicalRequest {
    /// The canonical request string, exactly as AWS defines it.
    pub fn canonical(&self) -> (String, String) {
        let mut query: Vec<(String, String)> = self
            .query
            .iter()
            .map(|(k, v)| (uri_encode(k, true), uri_encode(v, true)))
            .collect();
        query.sort();
        let canonical_query = query
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        // Header names lowercase, values trimmed, sorted by name.
        let mut headers: Vec<(String, String)> = self
            .headers
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        headers.sort();

        let canonical_headers = headers
            .iter()
            .map(|(k, v)| format!("{}:{}\n", k, v))
            .collect::<String>();
        let signed_headers = headers
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");

        let path = if self.path.is_empty() {
            "/".to_string()
        } else {
            uri_encode(&self.path, false)
        };

        let canonical = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            self.method.to_ascii_uppercase(),
            path,
            canonical_query,
            canonical_headers,
            signed_headers,
            sha256_hex(&self.payload)
        );
        (canonical, signed_headers)
    }
}

/// The result of signing: the headers to add to the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub authorization: String,
    pub signature: String,
    pub signed_headers: String,
}

/// Sign a request.
///
/// `timestamp` is `YYYYMMDDTHHMMSSZ`; the date scope is derived from it, so the
/// two can never disagree.
pub fn sign(
    req: &CanonicalRequest,
    creds: &Credentials,
    region: &str,
    service: &str,
    timestamp: &str,
) -> Signed {
    let date = &timestamp[..8];
    let scope = format!("{}/{}/{}/aws4_request", date, region, service);

    let mut req = req.clone();
    req.headers
        .insert("x-amz-date".to_string(), timestamp.to_string());
    if let Some(token) = &creds.session_token {
        req.headers
            .insert("x-amz-security-token".to_string(), token.clone());
    }

    let (canonical, signed_headers) = req.canonical();
    let string_to_sign = format!(
        "{}\n{}\n{}\n{}",
        ALGORITHM,
        timestamp,
        scope,
        sha256_hex(canonical.as_bytes())
    );

    // Derive the signing key: date, then region, then service, then a
    // terminator. Each step narrows what the key can sign.
    let k_date = hmac(
        format!("AWS4{}", creds.secret_access_key).as_bytes(),
        date.as_bytes(),
    );
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex(&hmac(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "{} Credential={}/{}, SignedHeaders={}, Signature={}",
        ALGORITHM, creds.access_key_id, scope, signed_headers, signature
    );

    Signed {
        authorization,
        signature,
        signed_headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS's published `get-vanilla` case from the SigV4 test suite.
    fn vanilla() -> (CanonicalRequest, Credentials) {
        let mut headers = BTreeMap::new();
        headers.insert("host".to_string(), "example.amazonaws.com".to_string());
        (
            CanonicalRequest {
                method: "GET".into(),
                path: "/".into(),
                query: vec![],
                headers,
                payload: Vec::new(),
            },
            Credentials {
                access_key_id: "AKIDEXAMPLE".into(),
                secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
                session_token: None,
            },
        )
    }

    #[test]
    fn matches_the_published_get_vanilla_vector() {
        let (req, creds) = vanilla();
        let signed = sign(&req, &creds, "us-east-1", "service", "20150830T123600Z");
        assert_eq!(
            signed.signature,
            "5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        assert_eq!(signed.signed_headers, "host;x-amz-date");
        assert!(signed.authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request"
        ));
    }

    #[test]
    fn the_canonical_request_has_the_shape_aws_specifies() {
        let (mut req, _) = vanilla();
        req.headers
            .insert("x-amz-date".into(), "20150830T123600Z".into());
        let (canonical, signed_headers) = req.canonical();
        let expected = "GET\n/\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\n\
                        host;x-amz-date\n\
                        e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(canonical, expected);
        assert_eq!(signed_headers, "host;x-amz-date");
    }

    #[test]
    fn the_empty_payload_hash_is_the_sha256_of_nothing() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn query_parameters_are_sorted_and_encoded() {
        let mut headers = BTreeMap::new();
        headers.insert("host".to_string(), "example.amazonaws.com".to_string());
        let req = CanonicalRequest {
            method: "GET".into(),
            path: "/".into(),
            query: vec![
                ("Param2".into(), "value2".into()),
                ("Param1".into(), "value1".into()),
            ],
            headers,
            payload: Vec::new(),
        };
        let (canonical, _) = req.canonical();
        assert!(
            canonical.contains("Param1=value1&Param2=value2"),
            "query must be sorted: {}",
            canonical
        );
    }

    #[test]
    fn slashes_are_encoded_in_queries_but_not_in_paths() {
        assert_eq!(uri_encode("a/b", false), "a/b");
        assert_eq!(uri_encode("a/b", true), "a%2Fb");
        assert_eq!(uri_encode("a b+c", true), "a%20b%2Bc");
        assert_eq!(uri_encode("~-._", true), "~-._", "unreserved stay literal");
    }

    #[test]
    fn header_values_are_trimmed_and_names_lowercased() {
        let mut headers = BTreeMap::new();
        headers.insert("Host".to_string(), "  example.amazonaws.com  ".to_string());
        let req = CanonicalRequest {
            method: "GET".into(),
            path: "/".into(),
            query: vec![],
            headers,
            payload: Vec::new(),
        };
        let (canonical, signed) = req.canonical();
        assert!(canonical.contains("host:example.amazonaws.com\n"));
        assert_eq!(signed, "host");
    }

    #[test]
    fn a_session_token_is_signed_when_present() {
        let (req, mut creds) = vanilla();
        creds.session_token = Some("session-token-value".into());
        let signed = sign(&req, &creds, "us-east-1", "service", "20150830T123600Z");
        assert!(
            signed.signed_headers.contains("x-amz-security-token"),
            "a session token must be covered by the signature: {}",
            signed.signed_headers
        );
    }

    #[test]
    fn a_different_payload_changes_the_signature() {
        let (mut req, creds) = vanilla();
        let a = sign(&req, &creds, "us-east-1", "service", "20150830T123600Z").signature;
        req.payload = b"something".to_vec();
        let b = sign(&req, &creds, "us-east-1", "service", "20150830T123600Z").signature;
        assert_ne!(a, b, "the body must be covered by the signature");
    }

    #[test]
    fn region_service_and_time_all_change_the_signature() {
        let (req, creds) = vanilla();
        let base = sign(&req, &creds, "us-east-1", "service", "20150830T123600Z").signature;
        assert_ne!(
            base,
            sign(&req, &creds, "eu-west-1", "service", "20150830T123600Z").signature
        );
        assert_ne!(
            base,
            sign(&req, &creds, "us-east-1", "sts", "20150830T123600Z").signature
        );
        assert_ne!(
            base,
            sign(&req, &creds, "us-east-1", "service", "20150831T123600Z").signature
        );
    }

    #[test]
    fn the_secret_key_is_not_recoverable_from_the_authorization_header() {
        let (req, creds) = vanilla();
        let signed = sign(&req, &creds, "us-east-1", "service", "20150830T123600Z");
        assert!(!signed.authorization.contains(&creds.secret_access_key));
        assert!(
            signed.authorization.contains(&creds.access_key_id),
            "the key id is public and must be sent"
        );
    }
}
