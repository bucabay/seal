//! Which project may use which reference.
//!
//! Without this, any `.keymaker` anywhere may name any reference: drop a
//! manifest in a scratch directory binding `stripe/sk_live` and the broker
//! resolves it. An agent jailed in one project could reach every credential on
//! the machine.
//!
//! A grant binds a reference to a **canonical directory path**, and nothing
//! else. Not a name, not an id in the manifest — a manifest is an ordinary
//! file that anyone can write, so anything inside it could be forged to
//! inherit another project's grants. The working directory is what the kernel
//! attests about the caller, so it is the only thing worth keying on.
//!
//! Grants are durable until revoked, which is what separates them from the
//! one-shot approvals in [`crate::approvals`]: "this project uses this key" is
//! a standing fact, "allow this particular refund" is not.

use crate::clock::Clock;
use crate::id::{Entropy, Id};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub reference: String,
    /// Canonical absolute path. Covers this directory and everything below it.
    pub project: PathBuf,
    pub granted_at: u64,
}

/// A use that has not been granted yet, waiting for a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub reference: String,
    pub project: PathBuf,
    pub requested_at: u64,
    /// What wanted it, for context — a task or endpoint name.
    #[serde(default)]
    pub wanted_by: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    grants: Vec<Grant>,
    #[serde(default)]
    pending: Vec<Request>,
}

/// Resolve a path for comparison: absolute, symlinks followed, `..` removed.
///
/// Without this, `/tmp` and `/private/tmp` are different projects on macOS, and
/// a symlink into a granted directory would sidestep the grant entirely.
pub fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        // The directory may not exist yet — a grant can be written before the
        // project is created. Normalise what can be normalised.
        let mut out = PathBuf::new();
        for part in path.components() {
            match part {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    })
}

/// Is `child` the same directory as `parent`, or inside it?
///
/// Compared component by component. A string prefix test would let `/a/bc`
/// pass a grant on `/a/b`.
pub fn is_within(child: &Path, parent: &Path) -> bool {
    let mut c = child.components();
    for part in parent.components() {
        match c.next() {
            Some(mine) if mine == part => {}
            _ => return false,
        }
    }
    true
}

#[derive(Debug)]
pub struct Grants<'a> {
    path: PathBuf,
    clock: &'a dyn Clock,
    entropy: &'a dyn Entropy,
}

impl<'a> Grants<'a> {
    pub fn new(path: impl Into<PathBuf>, clock: &'a dyn Clock, entropy: &'a dyn Entropy) -> Self {
        Grants {
            path: path.into(),
            clock,
            entropy,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> File {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn store(&self, file: &File) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(file) {
            // A failed write fails closed: the grant is simply not recorded, so
            // the use stays blocked.
            let _ = std::fs::write(&self.path, text);
        }
    }

    /// May a caller working in `cwd` use `reference`?
    ///
    /// An unknown working directory is never allowed. The whole mechanism rests
    /// on knowing where the caller is, so "we could not tell" has to mean no.
    pub fn allows(&self, reference: &str, cwd: Option<&Path>) -> bool {
        let Some(cwd) = cwd else { return false };
        let cwd = canonical(cwd);
        self.load()
            .grants
            .iter()
            .any(|g| g.reference == reference && is_within(&cwd, &g.project))
    }

    /// Record that a use is waiting to be granted. Returns the request id.
    ///
    /// An identical request already waiting is reused rather than stacked, so
    /// an agent retrying cannot bury the person in prompts.
    pub fn request(&self, reference: &str, project: &Path, wanted_by: &str) -> String {
        let project = canonical(project);
        let mut file = self.load();

        if let Some(existing) = file
            .pending
            .iter()
            .find(|r| r.reference == reference && r.project == project)
        {
            return existing.id.clone();
        }

        let id = Id::generate(self.entropy).to_string();
        file.pending.push(Request {
            id: id.clone(),
            reference: reference.to_string(),
            project,
            requested_at: self.clock.now(),
            wanted_by: wanted_by.to_string(),
        });
        self.store(&file);
        id
    }

    pub fn pending(&self) -> Vec<Request> {
        self.load().pending
    }

    pub fn all(&self) -> Vec<Grant> {
        self.load().grants
    }

    /// Everything granted to one project, for showing what it may reach.
    pub fn for_project(&self, project: &Path) -> Vec<Grant> {
        let project = canonical(project);
        self.load()
            .grants
            .into_iter()
            .filter(|g| is_within(&project, &g.project))
            .collect()
    }

    /// Every project that may use a reference, for showing the blast radius.
    pub fn projects_for(&self, reference: &str) -> Vec<PathBuf> {
        self.load()
            .grants
            .into_iter()
            .filter(|g| g.reference == reference)
            .map(|g| g.project)
            .collect()
    }

    /// Approve a waiting request. Returns false if it is not waiting.
    pub fn approve(&self, id: &str) -> bool {
        let mut file = self.load();
        let Some(idx) = file.pending.iter().position(|r| r.id == id) else {
            return false;
        };
        let request = file.pending.remove(idx);
        self.insert(&mut file, &request.reference, &request.project);
        self.store(&file);
        true
    }

    /// Refuse a waiting request, removing it without granting.
    pub fn deny(&self, id: &str) -> bool {
        let mut file = self.load();
        let before = file.pending.len();
        file.pending.retain(|r| r.id != id);
        let removed = file.pending.len() != before;
        if removed {
            self.store(&file);
        }
        removed
    }

    /// Grant directly, without a request having been made — what the GUI does
    /// when a person assembles a project's set of keys.
    pub fn grant(&self, reference: &str, project: &Path) {
        let project = canonical(project);
        let mut file = self.load();
        // A request for this pair is answered by granting it.
        file.pending
            .retain(|r| !(r.reference == reference && r.project == project));
        self.insert(&mut file, reference, &project);
        self.store(&file);
    }

    fn insert(&self, file: &mut File, reference: &str, project: &Path) {
        if file
            .grants
            .iter()
            .any(|g| g.reference == reference && g.project == project)
        {
            return;
        }
        file.grants.push(Grant {
            reference: reference.to_string(),
            project: project.to_path_buf(),
            granted_at: self.clock.now(),
        });
    }

    /// Take a grant away. Returns false if there was nothing to take.
    pub fn revoke(&self, reference: &str, project: &Path) -> bool {
        let project = canonical(project);
        let mut file = self.load();
        let before = file.grants.len();
        file.grants
            .retain(|g| !(g.reference == reference && g.project == project));
        let removed = file.grants.len() != before;
        if removed {
            self.store(&file);
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::id::SeqEntropy;

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "km-grants-{}-{}-{}",
                std::process::id(),
                n,
                tag
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        fn file(&self) -> PathBuf {
            self.0.join("grants.json")
        }
        fn project(&self, name: &str) -> PathBuf {
            let p = self.0.join(name);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_reference_is_refused_until_it_is_granted() {
        let d = Dir::new("refuse");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        assert!(!g.allows("stripe/sk_live", Some(&project)));
        g.grant("stripe/sk_live", &project);
        assert!(g.allows("stripe/sk_live", Some(&project)));
    }

    #[test]
    fn a_grant_does_not_leak_to_another_project() {
        let d = Dir::new("isolate");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let mine = d.project("mailkite");
        let other = d.project("somebody-else");

        g.grant("stripe/sk_live", &mine);
        assert!(g.allows("stripe/sk_live", Some(&mine)));
        assert!(
            !g.allows("stripe/sk_live", Some(&other)),
            "a manifest elsewhere must not reach a granted key"
        );
    }

    #[test]
    fn a_grant_covers_subdirectories_because_agents_run_from_them() {
        let d = Dir::new("subtree");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");
        let nested = d.project("mailkite/api/src");

        g.grant("stripe/sk_live", &project);
        assert!(g.allows("stripe/sk_live", Some(&nested)));
    }

    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_inside() {
        // `/a/bc` must not pass a grant on `/a/b`. A string prefix test would
        // let it through.
        let d = Dir::new("prefix");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let granted = d.project("app");
        let sibling = d.project("app-staging");

        g.grant("stripe/sk_live", &granted);
        assert!(
            !g.allows("stripe/sk_live", Some(&sibling)),
            "app-staging is not inside app"
        );
    }

    #[test]
    fn a_parent_of_a_granted_directory_is_not_covered() {
        let d = Dir::new("parent");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        g.grant("stripe/sk_live", &project);
        assert!(
            !g.allows("stripe/sk_live", Some(&d.0)),
            "a grant flows down, never up"
        );
    }

    #[test]
    fn an_unknown_working_directory_is_never_allowed() {
        let d = Dir::new("nocwd");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        g.grant("stripe/sk_live", &d.project("mailkite"));

        assert!(
            !g.allows("stripe/sk_live", None),
            "not knowing where the caller is has to mean no"
        );
    }

    #[test]
    fn symlinked_and_relative_paths_resolve_to_the_same_project() {
        let d = Dir::new("canon");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");
        d.project("mailkite/api");

        g.grant("stripe/sk_live", &project);
        // The same directory named the long way round.
        let roundabout = project.join("api").join("..");
        assert!(g.allows("stripe/sk_live", Some(&roundabout)));
    }

    #[test]
    fn a_new_use_waits_for_a_person_and_is_then_granted() {
        let d = Dir::new("tofu");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        let id = g.request("stripe/sk_live", &project, "task deploy");
        assert!(!g.allows("stripe/sk_live", Some(&project)), "still waiting");

        let waiting = g.pending();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].reference, "stripe/sk_live");
        assert_eq!(waiting[0].wanted_by, "task deploy");

        assert!(g.approve(&id));
        assert!(g.allows("stripe/sk_live", Some(&project)));
        assert!(g.pending().is_empty());
    }

    #[test]
    fn a_grant_is_durable_unlike_an_approval() {
        let d = Dir::new("durable");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        g.grant("stripe/sk_live", &project);
        c.advance(60 * 60 * 24 * 365);
        assert!(
            g.allows("stripe/sk_live", Some(&project)),
            "a grant does not lapse; it is revoked"
        );
    }

    #[test]
    fn denying_leaves_the_use_blocked() {
        let d = Dir::new("deny");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        let id = g.request("stripe/sk_live", &project, "");
        assert!(g.deny(&id));
        assert!(!g.allows("stripe/sk_live", Some(&project)));
        assert!(g.pending().is_empty());
        assert!(!g.deny(&id), "answering twice does nothing");
    }

    #[test]
    fn retrying_does_not_stack_requests() {
        let d = Dir::new("retry");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        let first = g.request("stripe/sk_live", &project, "");
        let second = g.request("stripe/sk_live", &project, "");
        assert_eq!(first, second);
        assert_eq!(g.pending().len(), 1);
    }

    #[test]
    fn revoking_takes_the_key_away_again() {
        let d = Dir::new("revoke");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        g.grant("stripe/sk_live", &project);
        assert!(g.revoke("stripe/sk_live", &project));
        assert!(!g.allows("stripe/sk_live", Some(&project)));
        assert!(
            !g.revoke("stripe/sk_live", &project),
            "nothing left to take"
        );
    }

    #[test]
    fn granting_twice_does_not_duplicate() {
        let d = Dir::new("dupe");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        g.grant("stripe/sk_live", &project);
        g.grant("stripe/sk_live", &project);
        assert_eq!(g.all().len(), 1);
    }

    #[test]
    fn granting_answers_a_request_for_the_same_pair() {
        let d = Dir::new("answer");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let project = d.project("mailkite");

        g.request("stripe/sk_live", &project, "");
        g.grant("stripe/sk_live", &project);
        assert!(g.pending().is_empty(), "granting settles the request");
        assert!(g.allows("stripe/sk_live", Some(&project)));
    }

    #[test]
    fn both_directions_are_answerable_for_the_ui() {
        let d = Dir::new("views");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        let mailkite = d.project("mailkite");
        let hardroad = d.project("hardroad");

        g.grant("stripe/sk_live", &mailkite);
        g.grant("stripe/sk_live", &hardroad);
        g.grant("hardroad/db_url", &hardroad);

        // What may this project reach?
        let theirs: Vec<String> = g
            .for_project(&hardroad)
            .into_iter()
            .map(|x| x.reference)
            .collect();
        assert_eq!(theirs, vec!["stripe/sk_live", "hardroad/db_url"]);

        // And who can use this key?
        assert_eq!(g.projects_for("stripe/sk_live").len(), 2);
        assert_eq!(g.projects_for("hardroad/db_url").len(), 1);
    }

    #[test]
    fn the_file_holds_names_and_paths_and_no_values() {
        let d = Dir::new("novalues");
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);
        g.grant("stripe/sk_live", &d.project("mailkite"));

        let text = std::fs::read_to_string(d.file()).unwrap();
        for forbidden in ["sk_live_", "secret", "password"] {
            assert!(
                !text.contains(forbidden),
                "grants must carry no value: {}",
                text
            );
        }
    }

    #[test]
    fn a_corrupt_file_blocks_rather_than_opens() {
        let d = Dir::new("corrupt");
        std::fs::write(d.file(), "not json at all").unwrap();
        let c = FixedClock::new(1_000);
        let e = SeqEntropy::new();
        let g = Grants::new(d.file(), &c, &e);

        assert!(
            !g.allows("stripe/sk_live", Some(&d.project("mailkite"))),
            "an unreadable grant file must not allow everything"
        );
    }

    #[test]
    fn path_containment_is_by_component() {
        assert!(is_within(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(is_within(Path::new("/a/b"), Path::new("/a/b")));
        assert!(!is_within(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(!is_within(Path::new("/a"), Path::new("/a/b")));
        assert!(!is_within(Path::new("/x/y"), Path::new("/a/b")));
    }
}
