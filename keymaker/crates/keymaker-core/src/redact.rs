//! Output redaction.
//!
//! The broker knows every value it injected, so it can strip those values out
//! of what a task prints before the bytes reach the agent.
//!
//! This is an *accident* control and nothing more. It catches a task that
//! echoes a key, logs a connection string, or dumps its environment. It does
//! not stop a task that transforms the value first — `base64`, `rev`, or
//! printing it in two halves all defeat it, and no amount of pattern matching
//! closes that. See docs/PLAN-D.md.

/// Values shorter than this are not redacted: masking every occurrence of a
/// four-character token would mangle ordinary output without protecting
/// anything worth protecting.
pub const MIN_REDACTABLE_LEN: usize = 6;

pub const MARKER: &[u8] = b"[redacted]";

fn base64_variants(raw: &[u8]) -> Vec<Vec<u8>> {
    const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let encode = |alphabet: &[u8; 64], pad: bool| -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in raw.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            let idx = [
                (n >> 18) & 63,
                (n >> 12) & 63,
                (n >> 6) & 63,
                n & 63,
            ];
            let keep = chunk.len() + 1;
            for (i, ix) in idx.iter().enumerate() {
                if i < keep {
                    out.push(alphabet[*ix as usize]);
                } else if pad {
                    out.push(b'=');
                }
            }
        }
        out
    };

    vec![
        encode(STD, true),
        encode(STD, false),
        encode(URL, true),
        encode(URL, false),
    ]
}

fn percent_encoded(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &b in raw {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(b);
        } else {
            out.extend_from_slice(format!("%{:02X}", b).as_bytes());
        }
    }
    out
}

fn json_escaped(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for &b in raw {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'/' => out.extend_from_slice(b"\\/"),
            _ => out.push(b),
        }
    }
    out
}

/// Builds the set of byte sequences that stand for a secret.
fn needles_for(value: &str) -> Vec<Vec<u8>> {
    let raw = value.as_bytes();
    let mut needles: Vec<Vec<u8>> = vec![raw.to_vec()];
    needles.extend(base64_variants(raw));
    needles.push(percent_encoded(raw));
    needles.push(json_escaped(raw));

    // Keep only what is long enough to be worth masking, and drop duplicates
    // (a value with no special characters encodes to itself under several of
    // the schemes above).
    needles.retain(|n| n.len() >= MIN_REDACTABLE_LEN);
    needles.sort();
    needles.dedup();
    // Longest first, so that a value which is a prefix of its own encoding is
    // masked in the longer form rather than leaving a tail behind.
    needles.sort_by(|a, b| b.len().cmp(&a.len()));
    needles
}

#[derive(Debug, Clone, Default)]
pub struct Redactor {
    needles: Vec<Vec<u8>>,
}

impl Redactor {
    pub fn new<S: AsRef<str>>(values: &[S]) -> Self {
        let mut needles = Vec::new();
        for v in values {
            let v = v.as_ref();
            if v.len() < MIN_REDACTABLE_LEN {
                continue;
            }
            needles.extend(needles_for(v));
        }
        needles.sort();
        needles.dedup();
        needles.sort_by(|a, b| b.len().cmp(&a.len()));
        Redactor { needles }
    }

    pub fn is_empty(&self) -> bool {
        self.needles.is_empty()
    }

    /// Longest sequence this redactor looks for; a stream must hold back at
    /// least this many bytes minus one to catch a match split across chunks.
    pub fn window(&self) -> usize {
        self.needles.first().map(|n| n.len()).unwrap_or(0)
    }

    pub fn redact(&self, input: &[u8]) -> Vec<u8> {
        if self.needles.is_empty() {
            return input.to_vec();
        }
        let mut out = Vec::with_capacity(input.len());
        let mut i = 0;
        'outer: while i < input.len() {
            for n in &self.needles {
                if input[i..].starts_with(n) {
                    out.extend_from_slice(MARKER);
                    i += n.len();
                    continue 'outer;
                }
            }
            out.push(input[i]);
            i += 1;
        }
        out
    }

    /// True if any known value survives in `input`. Used to check files a task
    /// wrote, where the right response is to fail the run rather than silently
    /// rewrite somebody's file.
    pub fn detects(&self, input: &[u8]) -> bool {
        self.needles
            .iter()
            .any(|n| input.windows(n.len()).any(|w| w == n.as_slice()))
    }
}

/// Redacts a stream without letting a value slip through by landing on a chunk
/// boundary.
#[derive(Debug)]
pub struct StreamRedactor {
    inner: Redactor,
    carry: Vec<u8>,
}

impl StreamRedactor {
    pub fn new(inner: Redactor) -> Self {
        StreamRedactor { inner, carry: Vec::new() }
    }

    /// Feed a chunk; returns the bytes that are safe to release now.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.carry.extend_from_slice(chunk);
        let hold = self.inner.window().saturating_sub(1);
        if self.carry.len() <= hold {
            return Vec::new();
        }
        let split = self.carry.len() - hold;
        let ready: Vec<u8> = self.carry.drain(..split).collect();
        self.inner.redact(&ready)
    }

    /// Release whatever is still held back. Call once the task has exited.
    pub fn finish(&mut self) -> Vec<u8> {
        let rest: Vec<u8> = std::mem::take(&mut self.carry);
        self.inner.redact(&rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "sk_live_51H8xQ2abcdef";

    fn r() -> Redactor {
        Redactor::new(&[SECRET])
    }

    fn s(v: &[u8]) -> String {
        String::from_utf8_lossy(v).into_owned()
    }

    #[test]
    fn masks_a_raw_value() {
        let out = r().redact(format!("token={} done", SECRET).as_bytes());
        assert_eq!(s(&out), "token=[redacted] done");
        assert!(!s(&out).contains(SECRET));
    }

    #[test]
    fn masks_every_occurrence() {
        let out = r().redact(format!("{} and {}", SECRET, SECRET).as_bytes());
        assert_eq!(s(&out), "[redacted] and [redacted]");
    }

    #[test]
    fn masks_base64_of_the_value() {
        // What `echo -n "$X" | base64` produces.
        let b64 = "c2tfbGl2ZV81MUg4eFEyYWJjZGVm";
        let out = r().redact(format!("blob {} end", b64).as_bytes());
        assert_eq!(s(&out), "blob [redacted] end");
    }

    #[test]
    fn masks_base64_without_padding() {
        let padded = "c2tfbGl2ZV81MUg4eFEyYWJjZGVm";
        let unpadded = padded.trim_end_matches('=');
        let out = r().redact(unpadded.as_bytes());
        assert_eq!(s(&out), "[redacted]");
    }

    #[test]
    fn masks_percent_encoded_values() {
        let secret = "pa/ss wo+rd!";
        let red = Redactor::new(&[secret]);
        let encoded = "pa%2Fss%20wo%2Brd%21";
        assert_eq!(s(&red.redact(encoded.as_bytes())), "[redacted]");
    }

    #[test]
    fn masks_json_escaped_values() {
        let secret = "line1\nline2\"quoted\"";
        let red = Redactor::new(&[secret]);
        let escaped = r#"line1\nline2\"quoted\""#;
        assert_eq!(s(&red.redact(escaped.as_bytes())), "[redacted]");
    }

    #[test]
    fn leaves_unrelated_output_alone() {
        let input = b"all tests passed in 1.2s";
        assert_eq!(r().redact(input), input.to_vec());
    }

    #[test]
    fn short_values_are_not_redacted() {
        // Masking "abc" would destroy ordinary output for no benefit.
        let red = Redactor::new(&["abc"]);
        assert!(red.is_empty());
        assert_eq!(s(&red.redact(b"abc def abc")), "abc def abc");
    }

    #[test]
    fn an_empty_redactor_is_a_passthrough() {
        let red = Redactor::new::<&str>(&[]);
        assert_eq!(red.redact(b"anything"), b"anything".to_vec());
        assert_eq!(red.window(), 0);
    }

    #[test]
    fn detects_reports_presence_without_rewriting() {
        let red = r();
        assert!(red.detects(format!("X={}", SECRET).as_bytes()));
        assert!(!red.detects(b"nothing here"));
    }

    #[test]
    fn a_value_split_across_chunks_is_still_caught() {
        let mut stream = StreamRedactor::new(r());
        let (head, tail) = SECRET.split_at(7);

        let mut got = stream.push(format!("key={}", head).as_bytes());
        got.extend(stream.push(format!("{}\n", tail).as_bytes()));
        got.extend(stream.finish());

        let text = s(&got);
        assert_eq!(text, "key=[redacted]\n");
        assert!(!text.contains(SECRET));
    }

    #[test]
    fn a_value_split_one_byte_at_a_time_is_still_caught() {
        let mut stream = StreamRedactor::new(r());
        let mut got = Vec::new();
        for b in format!("v={}!", SECRET).bytes() {
            got.extend(stream.push(&[b]));
        }
        got.extend(stream.finish());
        assert_eq!(s(&got), "v=[redacted]!");
    }

    #[test]
    fn a_stream_with_no_secrets_passes_everything_through() {
        let mut stream = StreamRedactor::new(r());
        let mut got = stream.push(b"hello ");
        got.extend(stream.push(b"world"));
        got.extend(stream.finish());
        assert_eq!(s(&got), "hello world");
    }

    #[test]
    fn multiple_secrets_are_all_masked() {
        let red = Redactor::new(&["first_secret_value", "second_secret_value"]);
        let out = red.redact(b"a=first_secret_value b=second_secret_value");
        assert_eq!(s(&out), "a=[redacted] b=[redacted]");
    }

    #[test]
    fn documented_limit_transformed_values_survive() {
        // This is the honest boundary, asserted so nobody later claims more.
        // `rev` produces a string the redactor cannot know about.
        let reversed: String = SECRET.chars().rev().collect();
        let out = r().redact(reversed.as_bytes());
        assert_eq!(s(&out), reversed, "redaction cannot catch transformed values");
    }
}
