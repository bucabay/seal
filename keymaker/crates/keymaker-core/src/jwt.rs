//! RS256 assertions.
//!
//! Some exchanges will not take a secret at all: a GitHub App proves who it is
//! by signing a short-lived JWT with its private key, and receives an
//! installation token in return.
//!
//! Signing uses `ring`, which is already in the tree via rustls, rather than a
//! second RSA implementation. Only signing happens here — never decryption — so
//! the padding-oracle class of RSA problem does not arise.

use crate::error::{Error, Result};

/// base64url without padding, which is what JWT uses.
pub fn b64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, ix) in idx.iter().enumerate() {
            if i <= chunk.len() {
                out.push(ALPHABET[*ix as usize] as char);
            }
        }
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Pull the DER body out of a PEM document.
///
/// GitHub hands out either PKCS#1 (`BEGIN RSA PRIVATE KEY`) or PKCS#8
/// (`BEGIN PRIVATE KEY`), so which one it is has to be reported, not guessed.
pub fn pem_to_der(pem: &str) -> Result<(Vec<u8>, KeyFormat)> {
    let format = if pem.contains("BEGIN RSA PRIVATE KEY") {
        KeyFormat::Pkcs1
    } else if pem.contains("BEGIN PRIVATE KEY") {
        KeyFormat::Pkcs8
    } else {
        return Err(Error::Parse(
            "not a PEM private key (expected BEGIN RSA PRIVATE KEY or BEGIN PRIVATE KEY)".into(),
        ));
    };

    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    let der = b64_decode(&body).ok_or_else(|| Error::Parse("PEM body is not base64".into()))?;
    if der.is_empty() {
        return Err(Error::Parse("PEM body is empty".into()));
    }
    Ok((der, format))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFormat {
    Pkcs1,
    Pkcs8,
}

/// Build and sign an RS256 JWT.
///
/// `claims` is the payload as JSON. The header is fixed: this signs one
/// algorithm, so there is no `alg` for a caller to confuse.
#[cfg(feature = "jwt")]
pub fn sign_rs256(claims: &serde_json::Value, key_pem: &str) -> Result<String> {
    use ring::rand::SystemRandom;
    use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};

    let (der, format) = pem_to_der(key_pem)?;
    let key = match format {
        KeyFormat::Pkcs1 => RsaKeyPair::from_der(&der),
        KeyFormat::Pkcs8 => RsaKeyPair::from_pkcs8(&der),
    }
    .map_err(|e| Error::Parse(format!("not a usable RSA private key: {}", e)))?;

    let header = b64url(br#"{"alg":"RS256","typ":"JWT"}"#);
    let payload = b64url(
        serde_json::to_string(claims)
            .map_err(|e| Error::Parse(format!("claims are not encodable: {}", e)))?
            .as_bytes(),
    );
    let signing_input = format!("{}.{}", header, payload);

    let mut signature = vec![0u8; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        signing_input.as_bytes(),
        &mut signature,
    )
    .map_err(|_| Error::Store("could not sign the assertion".into()))?;

    Ok(format!("{}.{}", signing_input, b64url(&signature)))
}

#[cfg(not(feature = "jwt"))]
pub fn sign_rs256(_claims: &serde_json::Value, _key_pem: &str) -> Result<String> {
    Err(Error::Store(
        "this build cannot sign assertions (built without the `jwt` feature)".into(),
    ))
}

/// Seconds since the unix epoch for an RFC 3339 timestamp such as
/// `2026-09-16T12:00:00Z`.
///
/// Only the shape GitHub returns is accepted; anything else is an error rather
/// than a silently wrong time.
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let num = |a: usize, b: usize| s[a..b].parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let days = days_from_civil(y, m as u32, d as u32);
    let secs = days * 86_400 + hh * 3_600 + mm * 60 + ss;
    u64::try_from(secs).ok()
}

/// Howard Hinnant's days-from-civil. The companion of the inverse used for
/// SigV4 timestamps.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway 2048-bit key, generated for these tests and nothing else.
    const TEST_KEY: &str = include_str!("../tests/data/test_key.pem");

    #[test]
    fn base64url_matches_the_jwt_alphabet() {
        assert_eq!(b64url(b""), "");
        assert_eq!(b64url(b"f"), "Zg");
        assert_eq!(b64url(b"fo"), "Zm8");
        assert_eq!(b64url(b"foo"), "Zm9v");
        assert_eq!(b64url(b"foobar"), "Zm9vYmFy");
        // No padding, and `-`/`_` rather than `+`/`/`.
        let encoded = b64url(&[0xfb, 0xff, 0xfe]);
        assert!(!encoded.contains('='));
        assert!(!encoded.contains('+') && !encoded.contains('/'));
        assert_eq!(encoded, "-__-");
    }

    #[test]
    fn base64_round_trips() {
        for case in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"hello world",
            &[0u8, 255, 128],
        ] {
            assert_eq!(b64_decode(&b64url(case)).unwrap(), case, "{:?}", case);
        }
    }

    #[test]
    fn a_pem_key_is_recognised_and_decoded() {
        let (der, format) = pem_to_der(TEST_KEY).unwrap();
        assert!(!der.is_empty());
        assert!(matches!(format, KeyFormat::Pkcs1 | KeyFormat::Pkcs8));
    }

    #[test]
    fn something_that_is_not_a_key_is_refused() {
        for bad in [
            "hello",
            "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----",
            "-----BEGIN PRIVATE KEY-----\n-----END PRIVATE KEY-----",
        ] {
            assert!(pem_to_der(bad).is_err(), "`{}` should be refused", bad);
        }
    }

    #[cfg(feature = "jwt")]
    #[test]
    fn a_signed_assertion_has_three_parts_and_the_right_header() {
        let claims =
            serde_json::json!({ "iss": "12345", "iat": 1_700_000_000, "exp": 1_700_000_600 });
        let jwt = sign_rs256(&claims, TEST_KEY).unwrap();

        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);

        let header = String::from_utf8(b64_decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header, r#"{"alg":"RS256","typ":"JWT"}"#);

        let payload: serde_json::Value =
            serde_json::from_slice(&b64_decode(parts[1]).unwrap()).unwrap();
        assert_eq!(payload["iss"], "12345");
        assert_eq!(payload["exp"], 1_700_000_600);

        // 2048-bit key means a 256-byte signature.
        assert_eq!(b64_decode(parts[2]).unwrap().len(), 256);
    }

    #[cfg(feature = "jwt")]
    #[test]
    fn the_signature_covers_the_claims() {
        let a = sign_rs256(&serde_json::json!({ "iss": "1" }), TEST_KEY).unwrap();
        let b = sign_rs256(&serde_json::json!({ "iss": "2" }), TEST_KEY).unwrap();
        assert_ne!(
            a.split('.').nth(2),
            b.split('.').nth(2),
            "different claims must produce different signatures"
        );
    }

    #[cfg(feature = "jwt")]
    #[test]
    fn a_signature_verifies_against_the_public_key() {
        use ring::signature;
        let claims = serde_json::json!({ "iss": "12345" });
        let jwt = sign_rs256(&claims, TEST_KEY).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let sig = b64_decode(parts[2]).unwrap();

        let (der, format) = pem_to_der(TEST_KEY).unwrap();
        let key = match format {
            KeyFormat::Pkcs1 => signature::RsaKeyPair::from_der(&der),
            KeyFormat::Pkcs8 => signature::RsaKeyPair::from_pkcs8(&der),
        }
        .unwrap();
        let public = signature::UnparsedPublicKey::new(
            &signature::RSA_PKCS1_2048_8192_SHA256,
            key.public().as_ref(),
        );
        assert!(
            public.verify(signing_input.as_bytes(), &sig).is_ok(),
            "the assertion must verify against its own public key"
        );
    }

    #[test]
    fn rfc3339_timestamps_parse() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2015-08-30T12:36:00Z"), Some(1_440_938_160));
        // A leap day, where a naive conversion goes wrong.
        assert_eq!(parse_rfc3339("2024-02-29T00:00:00Z"), Some(1_709_164_800));
    }

    #[test]
    fn a_malformed_timestamp_is_rejected_rather_than_guessed() {
        for bad in [
            "",
            "2026-09-16",
            "not a date",
            "2026/09/16T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-09-16T25:00:00Z",
        ] {
            assert!(parse_rfc3339(bad).is_none(), "`{}` should not parse", bad);
        }
    }

    #[test]
    fn the_date_conversions_are_inverses() {
        // Against the function used for SigV4 timestamps, over a long span.
        for days in [0i64, 1, 365, 10_000, 19_000, 25_000] {
            let secs = (days * 86_400) as u64;
            let formatted = crate::creds::HttpStsExchanger::amz_date(secs);
            let iso = format!(
                "{}-{}-{}T{}:{}:{}Z",
                &formatted[0..4],
                &formatted[4..6],
                &formatted[6..8],
                &formatted[9..11],
                &formatted[11..13],
                &formatted[13..15]
            );
            assert_eq!(
                parse_rfc3339(&iso),
                Some(secs),
                "round trip failed at {}",
                iso
            );
        }
    }
}
