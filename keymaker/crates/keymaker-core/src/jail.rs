//! Approach 1 — confine the agent.
//!
//! Rather than hiding secrets from the agent, take away its ability to reach
//! the places secrets live. Everything the agent spawns inherits the sandbox,
//! so it cannot shell out to escape.
//!
//! This is an enforcement layer, not a proof. It does not survive local root
//! outside the sandbox, and on macOS a denied operation is usually dropped
//! silently rather than returning an error, so a confined tool misbehaves
//! rather than failing loudly.

use serde::{Deserialize, Serialize};

/// Paths every profile denies. These are where credentials actually sit.
pub const CREDENTIAL_PATHS: &[&str] = &[
    "~/Library/Keychains",
    "~/.ssh",
    "~/.aws",
    "~/.config/gcloud",
    "~/.kube",
    "~/.docker/config.json",
    "~/.netrc",
    "~/.npmrc",
    "~/.pypirc",
    "~/.gnupg",
];

/// Tools that talk to a secret store on the agent's behalf. Denying the paths
/// above is not enough while these can be executed.
pub const CREDENTIAL_TOOLS: &[&str] = &[
    "/usr/bin/security",
    "/usr/bin/secret-tool",
    "/usr/bin/gpg",
    "/usr/local/bin/secret-tool",
];

/// Link-local metadata endpoints. Reaching these yields cloud credentials
/// without touching the filesystem at all.
pub const METADATA_HOSTS: &[&str] = &["169.254.169.254", "fd00:ec2::254", "metadata.google.internal"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Allow by default, deny the credential paths and tools. Practical: an
    /// agent keeps working, and the known-sensitive things are closed.
    Shield,
    /// Deny by default, allow only what is listed. Much stronger, and much
    /// more likely to break ordinary work.
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub mode: Mode,
    /// Absolute paths the agent may read and write. Only meaningful in strict
    /// mode; in shield mode everything not denied is already permitted.
    pub writable: Vec<String>,
    pub readable: Vec<String>,
    pub deny_read: Vec<String>,
    pub deny_exec: Vec<String>,
    pub deny_hosts: Vec<String>,
    /// Allow outbound network at all. A coding agent normally needs this.
    pub network: bool,
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/Users/unknown".into())
}

fn expand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("{}/{}", home().trim_end_matches('/'), rest),
        None => path.to_string(),
    }
}

/// Escape for an sbpl string literal.
fn sbpl_quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

impl Profile {
    /// The profile to use for a coding agent working in `workdir`.
    pub fn shield(workdir: &str) -> Profile {
        Profile {
            mode: Mode::Shield,
            writable: vec![workdir.to_string()],
            readable: Vec::new(),
            deny_read: CREDENTIAL_PATHS.iter().map(|p| expand(p)).collect(),
            deny_exec: CREDENTIAL_TOOLS.iter().map(|s| s.to_string()).collect(),
            deny_hosts: METADATA_HOSTS.iter().map(|s| s.to_string()).collect(),
            network: true,
        }
    }

    pub fn strict(workdir: &str) -> Profile {
        Profile {
            mode: Mode::Strict,
            writable: {
                let mut w = vec![workdir.to_string()];
                if workdir != "/tmp" {
                    w.push("/tmp".into());
                }
                w
            },
            readable: vec!["/usr".into(), "/bin".into(), "/sbin".into(), "/System".into()],
            deny_read: CREDENTIAL_PATHS.iter().map(|p| expand(p)).collect(),
            deny_exec: CREDENTIAL_TOOLS.iter().map(|s| s.to_string()).collect(),
            deny_hosts: METADATA_HOSTS.iter().map(|s| s.to_string()).collect(),
            network: true,
        }
    }

    /// Also deny the broker's own socket directory to everything but the
    /// client binary path given.
    pub fn deny_path(mut self, path: &str) -> Self {
        self.deny_read.push(expand(path));
        self
    }

    /// Render an Apple sandbox (seatbelt) profile.
    ///
    /// Deny rules are emitted *after* any blanket allow, because sbpl applies
    /// the last matching rule.
    pub fn to_seatbelt(&self) -> String {
        let mut p = String::from("(version 1)\n");
        match self.mode {
            Mode::Shield => p.push_str("(allow default)\n"),
            Mode::Strict => {
                p.push_str("(deny default)\n");
                p.push_str("(allow process-exec* process-fork signal)\n");
                p.push_str("(allow sysctl-read mach-lookup ipc-posix-shm)\n");
                // dyld maps the shared cache and stats the root on the way to
                // every library. Without these three a deny-default profile
                // loads cleanly and then aborts every process it wraps, which
                // looks like a crash rather than a policy decision.
                p.push_str("(allow file-map-executable file-read-metadata)\n");
                p.push_str("(allow file-read* (literal \"/\"))\n");
                for r in &self.readable {
                    p.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", sbpl_quote(r)));
                }
                for w in &self.writable {
                    p.push_str(&format!(
                        "(allow file-read* file-write* (subpath \"{}\"))\n",
                        sbpl_quote(w)
                    ));
                }
                if self.network {
                    p.push_str("(allow network-outbound)\n");
                }
            }
        }

        p.push_str("\n; credential stores\n");
        for d in &self.deny_read {
            p.push_str(&format!("(deny file-read* (subpath \"{}\"))\n", sbpl_quote(d)));
        }
        p.push_str("\n; tools that read credential stores\n");
        for d in &self.deny_exec {
            p.push_str(&format!(
                "(deny process-exec* (literal \"{}\"))\n",
                sbpl_quote(d)
            ));
        }
        if !self.deny_hosts.is_empty() {
            // Seatbelt's `remote ip` filter accepts only `*` or `localhost` as
            // the host part, so a specific address cannot be denied here. The
            // rule is recorded as a comment and enforced on Linux by a network
            // namespace instead; see `to_landlock_plan`.
            p.push_str("\n; cloud instance metadata — not expressible in sbpl, see PLAN.md\n");
            for h in &self.deny_hosts {
                p.push_str(&format!("; would deny: {}\n", h.replace('\n', " ")));
            }
        }
        p
    }

    /// Whether this platform's sandbox can deny a specific remote address.
    /// macOS cannot, so callers must not claim metadata endpoints are blocked
    /// there.
    pub const fn blocks_hosts() -> bool {
        !cfg!(target_os = "macos")
    }

    /// The equivalent plan for Linux. Landlock covers the filesystem, seccomp
    /// the syscalls, and a network namespace the metadata endpoints; this
    /// describes what each must enforce.
    pub fn to_landlock_plan(&self) -> LandlockPlan {
        LandlockPlan {
            read_write: self.writable.clone(),
            read_only: self.readable.clone(),
            denied: self.deny_read.clone(),
            denied_exec: self.deny_exec.clone(),
            blocked_hosts: self.deny_hosts.clone(),
            unshare_net: !self.deny_hosts.is_empty(),
        }
    }

    /// argv that runs `command` under this profile, or `None` where the
    /// platform has no supported mechanism.
    pub fn wrap_command(&self, command: &[String]) -> Option<Vec<String>> {
        if cfg!(target_os = "macos") {
            let mut argv = vec![
                "sandbox-exec".to_string(),
                "-p".to_string(),
                self.to_seatbelt(),
                "--".to_string(),
            ];
            argv.extend(command.iter().cloned());
            Some(argv)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandlockPlan {
    pub read_write: Vec<String>,
    pub read_only: Vec<String>,
    pub denied: Vec<String>,
    pub denied_exec: Vec<String>,
    pub blocked_hosts: Vec<String>,
    /// Landlock has no network rules for addresses, so the metadata block
    /// needs its own namespace.
    pub unshare_net: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shield_denies_every_credential_path() {
        let p = Profile::shield("/work");
        let sb = p.to_seatbelt();
        for path in ["Library/Keychains", ".ssh", ".aws", ".gnupg", ".netrc"] {
            assert!(sb.contains(path), "profile must deny {}", path);
        }
    }

    #[test]
    fn shield_denies_the_tools_that_read_the_store() {
        let sb = Profile::shield("/work").to_seatbelt();
        assert!(sb.contains("/usr/bin/security"));
        assert!(sb.contains("/usr/bin/secret-tool"));
        assert!(sb.contains("process-exec*"));
    }

    #[test]
    fn metadata_blocking_is_recorded_but_not_claimed_on_macos() {
        let sb = Profile::shield("/work").to_seatbelt();
        assert!(sb.contains("169.254.169.254"), "the intent is recorded");
        assert!(
            !sb.contains("(deny network-outbound (remote ip"),
            "sbpl rejects a specific address in `remote ip`; emitting one makes \
             the whole profile fail to load"
        );
        assert_eq!(Profile::blocks_hosts(), !cfg!(target_os = "macos"));
    }

    /// The real check: a profile that does not load protects nothing. Asserting
    /// on its text is not enough — an earlier version contained every expected
    /// substring and was still rejected by `sandbox-exec`.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_generated_profile_is_accepted_by_sandbox_exec() {
        for profile in [Profile::shield("/tmp"), Profile::strict("/tmp")] {
            let argv = profile.wrap_command(&["/usr/bin/true".to_string()]).unwrap();
            let out = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .output()
                .expect("run sandbox-exec");
            assert!(
                out.status.success(),
                "profile did not load, or the wrapped process could not start \
                 (exit {:?}): {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// And the profile must actually stop what it claims to stop.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_shield_profile_denies_reading_the_keychain_directory() {
        let profile = Profile::shield("/tmp");
        let home = std::env::var("HOME").unwrap();
        let argv = profile
            .wrap_command(&[
                "/bin/ls".to_string(),
                format!("{}/Library/Keychains", home),
            ])
            .unwrap();
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .expect("run sandbox-exec");
        assert!(
            !out.status.success(),
            "the keychain directory was readable inside the jail"
        );
    }

    #[test]
    fn denies_come_after_the_blanket_allow() {
        // sbpl applies the last matching rule, so ordering is load-bearing.
        let sb = Profile::shield("/work").to_seatbelt();
        let allow = sb.find("(allow default)").expect("allow default");
        let deny = sb.find("(deny file-read*").expect("a deny rule");
        assert!(allow < deny, "a deny before the blanket allow would be overridden");
    }

    #[test]
    fn strict_denies_by_default_and_allows_the_workdir() {
        let sb = Profile::strict("/work").to_seatbelt();
        assert!(sb.contains("(deny default)"));
        assert!(sb.contains("(allow file-read* file-write* (subpath \"/work\"))"));
        assert!(!sb.contains("(allow default)"));
    }

    #[test]
    fn strict_grants_what_the_loader_needs() {
        // Found the hard way: without these a deny-default profile loads and
        // then every wrapped process dies with SIGABRT inside dyld.
        let sb = Profile::strict("/work").to_seatbelt();
        assert!(sb.contains("file-map-executable"));
        assert!(sb.contains("file-read-metadata"));
        assert!(sb.contains(r#"(allow file-read* (literal "/"))"#));
    }

    #[test]
    fn strict_does_not_list_the_workdir_twice() {
        let p = Profile::strict("/tmp");
        assert_eq!(p.writable, vec!["/tmp".to_string()]);
    }

    #[test]
    fn home_relative_paths_are_expanded() {
        let p = Profile::shield("/work");
        assert!(
            p.deny_read.iter().all(|d| d.starts_with('/')),
            "every denied path must be absolute: {:?}",
            p.deny_read
        );
    }

    /// Counts quotes that actually open or close a literal, skipping any that
    /// the profile has escaped.
    fn unescaped_quotes(s: &str) -> usize {
        let mut count = 0;
        let mut escaped = false;
        for c in s.chars() {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => count += 1,
                _ => {}
            }
        }
        count
    }

    #[test]
    fn quotes_and_backslashes_in_paths_cannot_break_out_of_the_profile() {
        let p = Profile::shield("/work").deny_path("/tmp/we\"ird\\path");
        let sb = p.to_seatbelt();

        // The backslash is doubled and the quote escaped, so neither ends the
        // literal early.
        assert!(sb.contains(r#"/tmp/we\"ird\\path"#));

        // Every literal is still balanced once escapes are accounted for.
        assert_eq!(unescaped_quotes(&sb) % 2, 0);

        // And the hostile path did not introduce a rule of its own.
        assert!(!sb.contains("(allow file-read* (subpath \"/tmp/we"));
    }

    #[test]
    fn the_quote_counter_itself_is_right() {
        assert_eq!(unescaped_quotes(r#""a""#), 2);
        assert_eq!(unescaped_quotes(r#""a\"b""#), 2, "an escaped quote is not a delimiter");
        assert_eq!(unescaped_quotes(r#""a\\""#), 2, "an escaped backslash does not escape the quote");
    }

    #[test]
    fn an_extra_denied_path_is_included() {
        let p = Profile::shield("/work").deny_path("~/.keymaker");
        assert!(p.to_seatbelt().contains(".keymaker"));
    }

    #[test]
    fn the_landlock_plan_mirrors_the_seatbelt_profile() {
        let p = Profile::strict("/work");
        let plan = p.to_landlock_plan();
        assert!(plan.read_write.contains(&"/work".to_string()));
        assert_eq!(plan.denied, p.deny_read);
        assert_eq!(plan.denied_exec, p.deny_exec);
        assert!(plan.unshare_net, "metadata blocking needs its own namespace");
    }

    #[test]
    fn wrap_command_produces_a_runnable_argv_on_macos() {
        let p = Profile::shield("/work");
        let wrapped = p.wrap_command(&["claude".to_string(), "--help".to_string()]);
        if cfg!(target_os = "macos") {
            let argv = wrapped.expect("macos must produce an argv");
            assert_eq!(argv[0], "sandbox-exec");
            assert_eq!(argv[1], "-p");
            assert!(argv[2].contains("(version 1)"));
            assert_eq!(argv[3], "--");
            assert_eq!(&argv[4..], &["claude".to_string(), "--help".to_string()]);
        } else {
            assert!(wrapped.is_none());
        }
    }

    #[test]
    fn a_profile_round_trips_through_json() {
        let p = Profile::shield("/work");
        let text = serde_json::to_string(&p).unwrap();
        let back: Profile = serde_json::from_str(&text).unwrap();
        assert_eq!(p, back);
    }
}
