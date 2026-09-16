//! Catching a value that a task wrote to a file.
//!
//! Redaction only sees what a task *prints*. A task that writes a `.env`, a
//! debug log, or a config file puts the value somewhere redaction never looks,
//! and the agent can read it on its next turn.
//!
//! Because the agent is blocked while the task runs, there is a window in which
//! the broker is the only party that has seen what was written. This walks the
//! files touched during that window and reports any that contain an injected
//! value.
//!
//! Like redaction, this is an accident control. It catches a task that writes a
//! credential to disk to be helpful. It does not stop one that encodes the value
//! first, and it cannot see outside the roots it was given.

use crate::redact::Redactor;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Directories never worth walking: large, and not where a task writes a
/// credential by accident.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "dist",
    "build",
];

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// How deep to walk below each root.
    pub max_depth: usize,
    /// Stop after this many files, so a huge tree cannot stall a run.
    pub max_files: usize,
    /// Files larger than this are not read. A credential written by accident
    /// lands in a small file.
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: 6,
            max_files: 20_000,
            max_file_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: PathBuf,
}

/// Records when a run began so that only files touched since can be considered.
#[derive(Debug, Clone)]
pub struct Watcher {
    roots: Vec<PathBuf>,
    since: SystemTime,
    limits: Limits,
    /// True when the walk stopped early, so a caller can say "nothing found,
    /// but the search was incomplete" rather than implying a clean result.
    truncated: std::cell::Cell<bool>,
}

impl Watcher {
    /// Start watching. Call immediately before spawning the task.
    pub fn begin(roots: Vec<PathBuf>) -> Watcher {
        Watcher::begin_with(roots, Limits::default())
    }

    pub fn begin_with(roots: Vec<PathBuf>, limits: Limits) -> Watcher {
        Watcher {
            roots,
            // A file written in the same second the run started still counts,
            // so step back one second rather than miss it.
            since: SystemTime::now() - std::time::Duration::from_secs(1),
            limits,
            truncated: std::cell::Cell::new(false),
        }
    }

    /// For tests, and for a caller that already knows when the run began.
    pub fn since(roots: Vec<PathBuf>, since: SystemTime, limits: Limits) -> Watcher {
        Watcher {
            roots,
            since,
            limits,
            truncated: std::cell::Cell::new(false),
        }
    }

    pub fn was_truncated(&self) -> bool {
        self.truncated.get()
    }

    /// Files modified since the run began that contain a value the redactor
    /// knows about.
    pub fn findings(&self, redactor: &Redactor) -> Vec<Finding> {
        if redactor.is_empty() {
            return Vec::new();
        }
        let mut found = Vec::new();
        let mut budget = self.limits.max_files;
        for root in &self.roots {
            self.walk(root, 0, redactor, &mut found, &mut budget);
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found.dedup();
        found
    }

    fn walk(
        &self,
        dir: &Path,
        depth: usize,
        redactor: &Redactor,
        found: &mut Vec<Finding>,
        budget: &mut usize,
    ) {
        if depth > self.limits.max_depth {
            self.truncated.set(true);
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return; // unreadable directory is not a finding
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                self.truncated.set(true);
                return;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };

            if meta.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if SKIP_DIRS.contains(&name.as_ref()) {
                    continue;
                }
                // Do not follow symlinks: a link into $HOME would turn a scan
                // of the working directory into a scan of everything.
                if meta.file_type().is_symlink() {
                    continue;
                }
                self.walk(&path, depth + 1, redactor, found, budget);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            *budget -= 1;

            if meta.len() > self.limits.max_file_bytes {
                continue;
            }
            match meta.modified() {
                Ok(m) if m >= self.since => {}
                _ => continue,
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if redactor.detects(&bytes) {
                found.push(Finding { path });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "postgres://u:hunter2@db/prod";

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p =
                std::env::temp_dir().join(format!("km-scan-{}-{}-{}", std::process::id(), n, tag));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        fn write(&self, rel: &str, body: &str) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, body).unwrap();
            p
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn redactor() -> Redactor {
        Redactor::new(&[SECRET])
    }

    #[test]
    fn a_task_that_writes_a_dotenv_is_caught() {
        let d = Dir::new("dotenv");
        let w = Watcher::begin(vec![d.0.clone()]);
        let path = d.write(".env", &format!("DATABASE_URL={}\n", SECRET));

        let found = w.findings(&redactor());
        assert_eq!(found, vec![Finding { path }]);
    }

    #[test]
    fn a_value_in_a_nested_file_is_caught() {
        let d = Dir::new("nested");
        let w = Watcher::begin(vec![d.0.clone()]);
        let path = d.write("a/b/c/debug.log", &format!("connecting to {}", SECRET));

        assert_eq!(w.findings(&redactor()), vec![Finding { path }]);
    }

    #[test]
    fn files_without_a_value_are_not_reported() {
        let d = Dir::new("clean");
        let w = Watcher::begin(vec![d.0.clone()]);
        d.write("out.log", "all tests passed");
        d.write("README.md", "nothing to see");

        assert!(w.findings(&redactor()).is_empty());
    }

    #[test]
    fn a_file_written_before_the_run_is_ignored() {
        let d = Dir::new("pre-existing");
        // Already on disk, and already containing the value: not this run's
        // doing, so not this run's finding.
        d.write("old.env", SECRET);

        let w = Watcher::since(
            vec![d.0.clone()],
            SystemTime::now() + std::time::Duration::from_secs(60),
            Limits::default(),
        );
        assert!(
            w.findings(&redactor()).is_empty(),
            "only files touched during the run should be reported"
        );
    }

    #[test]
    fn an_encoded_value_is_not_caught_and_that_is_documented() {
        // The same honest limit as redaction: this is an accident control.
        let d = Dir::new("encoded");
        let w = Watcher::begin(vec![d.0.clone()]);
        let reversed: String = SECRET.chars().rev().collect();
        d.write("sneaky.txt", &reversed);

        assert!(
            w.findings(&redactor()).is_empty(),
            "scanning cannot catch a transformed value, and must not claim to"
        );
    }

    #[test]
    fn noisy_directories_are_skipped() {
        let d = Dir::new("skip");
        let w = Watcher::begin(vec![d.0.clone()]);
        d.write("node_modules/pkg/index.js", SECRET);
        d.write(".git/COMMIT_EDITMSG", SECRET);
        d.write("target/debug/build.log", SECRET);

        assert!(
            w.findings(&redactor()).is_empty(),
            "build and vcs directories are not where an accident matters"
        );
    }

    #[test]
    fn a_large_file_is_not_read() {
        let d = Dir::new("large");
        let limits = Limits {
            max_file_bytes: 64,
            ..Default::default()
        };
        let w = Watcher::since(vec![d.0.clone()], SystemTime::UNIX_EPOCH, limits);
        d.write("big.bin", &format!("{}{}", SECRET, "x".repeat(200)));

        assert!(w.findings(&redactor()).is_empty());
    }

    #[test]
    fn hitting_the_file_budget_is_reported_rather_than_looking_clean() {
        let d = Dir::new("budget");
        let limits = Limits {
            max_files: 2,
            ..Default::default()
        };
        let w = Watcher::since(vec![d.0.clone()], SystemTime::UNIX_EPOCH, limits);
        for i in 0..10 {
            d.write(&format!("f{}.txt", i), "harmless");
        }

        let found = w.findings(&redactor());
        assert!(found.is_empty());
        assert!(
            w.was_truncated(),
            "an incomplete search must not be mistaken for a clean one"
        );
    }

    #[test]
    fn depth_is_bounded_and_reported() {
        let d = Dir::new("depth");
        let limits = Limits {
            max_depth: 1,
            ..Default::default()
        };
        let w = Watcher::since(vec![d.0.clone()], SystemTime::UNIX_EPOCH, limits);
        d.write("a/b/c/d/e/deep.env", SECRET);

        assert!(w.findings(&redactor()).is_empty());
        assert!(w.was_truncated());
    }

    #[test]
    fn nothing_is_scanned_when_no_values_were_injected() {
        let d = Dir::new("empty-redactor");
        let w = Watcher::begin(vec![d.0.clone()]);
        d.write(".env", SECRET);

        let empty = Redactor::new::<&str>(&[]);
        assert!(w.findings(&empty).is_empty());
        assert!(!w.was_truncated());
    }

    #[test]
    fn a_missing_root_is_not_an_error() {
        let w = Watcher::begin(vec![PathBuf::from("/nonexistent/path/here")]);
        assert!(w.findings(&redactor()).is_empty());
    }

    #[test]
    fn each_file_is_reported_once_and_in_a_stable_order() {
        let d = Dir::new("order");
        let w = Watcher::begin(vec![d.0.clone(), d.0.clone()]); // same root twice
        let a = d.write("a.env", SECRET);
        let b = d.write("b.env", SECRET);

        let found = w.findings(&redactor());
        assert_eq!(
            found,
            vec![Finding { path: a }, Finding { path: b }],
            "a repeated root must not produce duplicates"
        );
    }
}
