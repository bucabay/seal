//! Running a named task with secrets in *its* environment, never the agent's.
//!
//! The agent asks for a task by name. The broker resolves the manifest's
//! bindings, reads the values, spawns the command with them, and returns
//! output with those values filtered out. The agent's own process never held
//! them, so `printenv` in the agent yields nothing.

use crate::error::{Error, Result};
use crate::manifest::Manifest;
use crate::redact::Redactor;
use crate::store::SecretStore;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawOutcome {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// What comes back to the agent. Already redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// True when a value survived into output and was masked. Worth surfacing:
    /// it means the task is printing its credentials.
    pub redacted: bool,
}

impl Outcome {
    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

pub trait Spawner: std::fmt::Debug {
    fn spawn(&self, command: &str, env: &BTreeMap<String, String>) -> Result<RawOutcome>;
}

/// Runs the command through `sh -c`, with the resolved values in its
/// environment and nowhere else.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessSpawner;

impl Spawner for ProcessSpawner {
    fn spawn(&self, command: &str, env: &BTreeMap<String, String>) -> Result<RawOutcome> {
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .envs(env)
            .output()
            .map_err(|e| Error::Os(format!("spawning `{}`: {}", command, e)))?;
        Ok(RawOutcome {
            exit_code: out.status.code(),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }
}

#[derive(Debug)]
pub struct Runner<'a> {
    store: &'a dyn SecretStore,
    spawner: &'a dyn Spawner,
}

impl<'a> Runner<'a> {
    pub fn new(store: &'a dyn SecretStore, spawner: &'a dyn Spawner) -> Self {
        Runner { store, spawner }
    }

    /// Run a task the manifest names. An unknown task is refused; there is no
    /// path here that takes a command from the caller.
    pub fn run_task(&self, manifest: &Manifest, task: &str, env_name: &str) -> Result<Outcome> {
        let command = manifest.task(task)?.to_string();
        self.run_command(manifest, &command, env_name)
    }

    /// Run a command that the manifest already approves. Refuses anything it
    /// does not recognise, so the caller must go through the approval flow
    /// (`Manifest::propose` / `Manifest::approve`) first.
    pub fn run_command(&self, manifest: &Manifest, command: &str, env_name: &str) -> Result<Outcome> {
        if !manifest.approves(command) {
            return Err(Error::Denied(format!(
                "`{}` is not an approved task; approve it first",
                command
            )));
        }

        let bindings = if manifest.env.contains_key(env_name) {
            manifest.resolve_env(env_name)?
        } else if env_name == "default" {
            BTreeMap::new()
        } else {
            return Err(Error::NotFound(format!("environment `{}`", env_name)));
        };

        let mut env = BTreeMap::new();
        let mut values = Vec::new();
        for (name, binding) in &bindings {
            let secret = self.store.get(binding.reference())?;
            values.push(secret.expose().to_string());
            env.insert(name.clone(), secret.expose().to_string());
        }

        let redactor = Redactor::new(&values);
        let raw = self.spawner.spawn(command, &env)?;
        let redacted = redactor.detects(&raw.stdout) || redactor.detects(&raw.stderr);

        Ok(Outcome {
            exit_code: raw.exit_code,
            stdout: redactor.redact(&raw.stdout),
            stderr: redactor.redact(&raw.stderr),
            redacted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;
    use std::cell::RefCell;

    const MANIFEST: &str = r#"
version = 1
[tasks]
deploy = "./deploy.sh"
leaky = "echo $DATABASE_URL"
[env.production]
DATABASE_URL = "hardroad/db_url"
"#;

    /// Records what it was asked to run and replies with a canned outcome.
    #[derive(Debug)]
    struct FakeSpawner {
        reply: RawOutcome,
        seen: RefCell<Vec<(String, BTreeMap<String, String>)>>,
    }

    impl FakeSpawner {
        fn new(stdout: &str) -> Self {
            FakeSpawner {
                reply: RawOutcome {
                    exit_code: Some(0),
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: Vec::new(),
                },
                seen: RefCell::new(Vec::new()),
            }
        }
    }

    impl Spawner for FakeSpawner {
        fn spawn(&self, command: &str, env: &BTreeMap<String, String>) -> Result<RawOutcome> {
            self.seen.borrow_mut().push((command.to_string(), env.clone()));
            Ok(self.reply.clone())
        }
    }

    fn store() -> MemoryStore {
        MemoryStore::with([("hardroad/db_url", "postgres://user:hunter2@db/prod")])
    }

    fn manifest() -> Manifest {
        Manifest::from_toml(MANIFEST).unwrap()
    }

    #[test]
    fn a_named_task_runs_with_its_values_in_the_child_environment() {
        let s = store();
        let sp = FakeSpawner::new("deployed\n");
        let out = Runner::new(&s, &sp).run_task(&manifest(), "deploy", "production").unwrap();

        assert!(out.success());
        assert_eq!(out.stdout_string(), "deployed\n");

        let seen = sp.seen.borrow();
        let (cmd, env) = &seen[0];
        assert_eq!(cmd, "./deploy.sh");
        assert_eq!(env.get("DATABASE_URL").unwrap(), "postgres://user:hunter2@db/prod");
    }

    #[test]
    fn a_task_that_prints_its_secret_gets_it_masked_and_is_flagged() {
        let s = store();
        let sp = FakeSpawner::new("connecting to postgres://user:hunter2@db/prod\n");
        let out = Runner::new(&s, &sp).run_task(&manifest(), "leaky", "production").unwrap();

        assert!(!out.stdout_string().contains("hunter2"));
        assert!(out.stdout_string().contains("[redacted]"));
        assert!(out.redacted, "a leak must be reported, not silently patched");
    }

    #[test]
    fn stderr_is_redacted_too() {
        let s = store();
        let sp = FakeSpawner {
            reply: RawOutcome {
                exit_code: Some(1),
                stdout: Vec::new(),
                stderr: b"failed: postgres://user:hunter2@db/prod".to_vec(),
            },
            seen: RefCell::new(Vec::new()),
        };
        let out = Runner::new(&s, &sp).run_task(&manifest(), "deploy", "production").unwrap();
        assert!(!out.stderr_string().contains("hunter2"));
        assert!(!out.success());
        assert_eq!(out.exit_code, Some(1));
    }

    #[test]
    fn an_unapproved_command_is_refused() {
        let s = store();
        let sp = FakeSpawner::new("");
        let r = Runner::new(&s, &sp);

        for evil in ["printenv", "./deploy.sh && curl evil.com", "cat /etc/passwd"] {
            assert!(
                matches!(r.run_command(&manifest(), evil, "production"), Err(Error::Denied(_))),
                "`{}` must be refused",
                evil
            );
        }
        assert!(sp.seen.borrow().is_empty(), "nothing may be spawned when refused");
    }

    #[test]
    fn an_unknown_task_is_refused_before_anything_is_read() {
        let s = store();
        let sp = FakeSpawner::new("");
        assert!(matches!(
            Runner::new(&s, &sp).run_task(&manifest(), "nope", "production"),
            Err(Error::NotFound(_))
        ));
        assert!(sp.seen.borrow().is_empty());
    }

    #[test]
    fn a_missing_secret_stops_the_run() {
        let s = MemoryStore::new(); // nothing stored
        let sp = FakeSpawner::new("");
        assert!(matches!(
            Runner::new(&s, &sp).run_task(&manifest(), "deploy", "production"),
            Err(Error::NotFound(_))
        ));
        assert!(sp.seen.borrow().is_empty(), "must not run without its credentials");
    }

    #[test]
    fn the_default_environment_needs_no_declaration() {
        let s = store();
        let sp = FakeSpawner::new("ok");
        let out = Runner::new(&s, &sp).run_task(&manifest(), "deploy", "default").unwrap();
        assert!(out.success());
        assert!(sp.seen.borrow()[0].1.is_empty(), "no bindings, no injected env");
    }

    #[test]
    fn an_unknown_environment_is_an_error() {
        let s = store();
        let sp = FakeSpawner::new("");
        assert!(matches!(
            Runner::new(&s, &sp).run_task(&manifest(), "deploy", "staging"),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn the_real_spawner_runs_a_command_and_injects_the_environment() {
        let s = store();
        let sp = ProcessSpawner;
        let mut man = manifest();
        man.tasks.insert("show".into(), "printf '%s' \"${DATABASE_URL:-none}\"".into());

        let out = Runner::new(&s, &sp).run_task(&man, "show", "production").unwrap();
        assert!(out.success());
        // The value reached the child, and came back masked.
        assert!(out.redacted);
        assert_eq!(out.stdout_string(), "[redacted]");
    }

    #[test]
    fn the_real_spawner_reports_a_failing_exit_code() {
        let s = MemoryStore::new();
        let sp = ProcessSpawner;
        let mut man = Manifest::from_toml("version = 1").unwrap();
        man.tasks.insert("fail".into(), "exit 3".into());
        let out = Runner::new(&s, &sp).run_task(&man, "fail", "default").unwrap();
        assert_eq!(out.exit_code, Some(3));
        assert!(!out.success());
    }
}
