//! Linux enforcement for the agent jail.
//!
//! Landlock only ever *restricts*: a ruleset grants access to specific paths,
//! and anything not granted is denied. There is no "deny this one directory"
//! rule.
//!
//! So "allow everything except `~/.ssh`" has to be turned inside out — grant
//! every sibling of the denied paths, and simply never grant the denied ones.
//! That computation is the interesting part and is tested on every platform;
//! the syscalls that apply it are Linux-only.
//!
//! What this does not do: block the cloud metadata endpoint. Landlock has no
//! network-address rules, and isolating one address needs a network namespace
//! with a packet filter, which would also cut off the network the agent
//! legitimately needs. Same honest gap as macOS — see `Profile::blocks_hosts`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Lists the entries of a directory. Injected so the allow-list computation can
/// be tested against a known tree rather than whatever the machine happens to
/// have.
pub trait DirLister {
    fn entries(&self, dir: &Path) -> Vec<PathBuf>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RealDirs;

impl DirLister for RealDirs {
    fn entries(&self, dir: &Path) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        out.sort();
        out
    }
}

/// Turn a deny-list into the grant-list Landlock actually needs.
///
/// For each denied path, every sibling is granted, and the denied entry is not.
/// Walking up from the denied path to the filesystem root means a denied path
/// several levels down (`~/.config/gcloud`) still leaves the rest of `~/.config`
/// reachable.
pub fn allow_list(denied: &[PathBuf], lister: &dyn DirLister) -> Vec<PathBuf> {
    let denied: BTreeSet<PathBuf> = denied.iter().cloned().collect();
    let mut granted: BTreeSet<PathBuf> = BTreeSet::new();

    // Every ancestor directory of something denied has to be expanded into its
    // children, so that the denied child can be left out.
    let mut to_expand: BTreeSet<PathBuf> = BTreeSet::new();
    for d in &denied {
        let mut cur = d.parent().map(Path::to_path_buf);
        while let Some(dir) = cur {
            to_expand.insert(dir.clone());
            if dir.parent().is_none() {
                break;
            }
            cur = dir.parent().map(Path::to_path_buf);
        }
    }
    // If nothing is denied, grant the root and stop.
    if to_expand.is_empty() {
        granted.insert(PathBuf::from("/"));
        return granted.into_iter().collect();
    }

    for dir in &to_expand {
        for child in lister.entries(dir) {
            if denied.contains(&child) {
                continue;
            }
            // A child that is itself on the way to something denied must be
            // expanded rather than granted wholesale.
            if to_expand.contains(&child) {
                continue;
            }
            granted.insert(child);
        }
    }

    // Drop any grant that a broader grant already covers.
    let all: Vec<PathBuf> = granted.iter().cloned().collect();
    all.iter()
        .filter(|p| !all.iter().any(|other| other != *p && p.starts_with(other)))
        .cloned()
        .collect()
}

/// Does `granted` let a process reach `path`?
pub fn reachable(granted: &[PathBuf], path: &Path) -> bool {
    granted.iter().any(|g| path.starts_with(g))
}

#[cfg(target_os = "linux")]
mod enforce {
    use super::*;

    // Syscall numbers are stable ABI on Linux.
    const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
    const SYS_LANDLOCK_ADD_RULE: libc::c_long = 445;
    const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;

    const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;
    const LANDLOCK_RULE_PATH_BENEATH: libc::c_int = 1;

    // ABI 1 access rights. Later ABIs add more; asking for a right the running
    // kernel does not know about makes ruleset creation fail, so this sticks to
    // the set every Landlock kernel has.
    const ACCESS_FS_ABI1: u64 = (1 << 0)  // execute
        | (1 << 1)  // write_file
        | (1 << 2)  // read_file
        | (1 << 3)  // read_dir
        | (1 << 4)  // remove_dir
        | (1 << 5)  // remove_file
        | (1 << 6)  // make_char
        | (1 << 7)  // make_dir
        | (1 << 8)  // make_reg
        | (1 << 9)  // make_sock
        | (1 << 10) // make_fifo
        | (1 << 11) // make_block
        | (1 << 12); // make_sym

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    /// The Landlock ABI this kernel supports, or `None` if there is none.
    pub fn abi_version() -> Option<i32> {
        // SAFETY: with the VERSION flag the kernel ignores the attr pointer.
        let rc = unsafe {
            libc::syscall(
                SYS_LANDLOCK_CREATE_RULESET,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        (rc > 0).then_some(rc as i32)
    }

    /// Restrict this process to `granted`, irreversibly, for it and every
    /// child it spawns.
    pub fn apply(granted: &[PathBuf]) -> Result<(), String> {
        if abi_version().is_none() {
            return Err("this kernel has no Landlock support".into());
        }

        // Landlock requires no_new_privs: without it a setuid binary could
        // escape the restriction.
        // SAFETY: prctl with these arguments only sets a flag on this process.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err("could not set no_new_privs".into());
        }

        let attr = RulesetAttr {
            handled_access_fs: ACCESS_FS_ABI1,
        };
        // SAFETY: `attr` is a valid, correctly sized ruleset attribute.
        let ruleset_fd = unsafe {
            libc::syscall(
                SYS_LANDLOCK_CREATE_RULESET,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        };
        if ruleset_fd < 0 {
            return Err("could not create a Landlock ruleset".into());
        }
        let ruleset_fd = ruleset_fd as libc::c_int;

        for path in granted {
            let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
                continue;
            };
            // SAFETY: c_path is a valid NUL-terminated path.
            let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
            if fd < 0 {
                continue; // a path that does not exist needs no rule
            }
            let rule = PathBeneathAttr {
                allowed_access: ACCESS_FS_ABI1,
                parent_fd: fd,
            };
            // SAFETY: `rule` is a valid path_beneath attribute and `fd` is open.
            let rc = unsafe {
                libc::syscall(
                    SYS_LANDLOCK_ADD_RULE,
                    ruleset_fd,
                    LANDLOCK_RULE_PATH_BENEATH,
                    &rule as *const PathBeneathAttr,
                    0u32,
                )
            };
            // SAFETY: fd was opened above and is not used again.
            unsafe { libc::close(fd) };
            if rc != 0 {
                // SAFETY: ruleset_fd is open.
                unsafe { libc::close(ruleset_fd) };
                return Err(format!("could not add a rule for {}", path.display()));
            }
        }

        // SAFETY: ruleset_fd is a valid ruleset descriptor.
        let rc = unsafe { libc::syscall(SYS_LANDLOCK_RESTRICT_SELF, ruleset_fd, 0u32) };
        // SAFETY: ruleset_fd is open and no longer needed either way.
        unsafe { libc::close(ruleset_fd) };
        if rc != 0 {
            return Err("could not apply the Landlock ruleset".into());
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub use enforce::{abi_version, apply};

#[cfg(not(target_os = "linux"))]
pub fn abi_version() -> Option<i32> {
    None
}

#[cfg(not(target_os = "linux"))]
pub fn apply(_granted: &[PathBuf]) -> Result<(), String> {
    Err("Landlock is Linux-only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed tree, so the computation is tested rather than the machine.
    struct FakeTree;

    impl DirLister for FakeTree {
        fn entries(&self, dir: &Path) -> Vec<PathBuf> {
            let listing: &[(&str, &[&str])] = &[
                ("/", &["/bin", "/etc", "/home", "/tmp", "/usr", "/var"]),
                ("/home", &["/home/gabe", "/home/other"]),
                (
                    "/home/gabe",
                    &[
                        "/home/gabe/.aws",
                        "/home/gabe/.config",
                        "/home/gabe/.ssh",
                        "/home/gabe/code",
                        "/home/gabe/notes.txt",
                    ],
                ),
                (
                    "/home/gabe/.config",
                    &["/home/gabe/.config/fish", "/home/gabe/.config/gcloud"],
                ),
            ];
            listing
                .iter()
                .find(|(d, _)| Path::new(d) == dir)
                .map(|(_, kids)| kids.iter().map(PathBuf::from).collect())
                .unwrap_or_default()
        }
    }

    fn granted_for(denied: &[&str]) -> Vec<PathBuf> {
        let d: Vec<PathBuf> = denied.iter().map(PathBuf::from).collect();
        allow_list(&d, &FakeTree)
    }

    #[test]
    fn a_denied_directory_is_not_reachable_but_its_siblings_are() {
        let granted = granted_for(&["/home/gabe/.ssh"]);

        assert!(!reachable(&granted, Path::new("/home/gabe/.ssh")));
        assert!(!reachable(&granted, Path::new("/home/gabe/.ssh/id_rsa")));

        assert!(reachable(&granted, Path::new("/home/gabe/code")));
        assert!(reachable(&granted, Path::new("/home/gabe/notes.txt")));
        assert!(reachable(&granted, Path::new("/usr/bin/git")));
        assert!(reachable(&granted, Path::new("/tmp/scratch")));
    }

    #[test]
    fn several_denied_paths_are_all_excluded() {
        let granted = granted_for(&[
            "/home/gabe/.ssh",
            "/home/gabe/.aws",
            "/home/gabe/.config/gcloud",
        ]);

        for denied in [
            "/home/gabe/.ssh",
            "/home/gabe/.aws",
            "/home/gabe/.config/gcloud",
        ] {
            assert!(
                !reachable(&granted, Path::new(denied)),
                "{} should be unreachable",
                denied
            );
        }
        // And the rest of the tree survives, including the siblings of a
        // deeply nested denial.
        assert!(reachable(&granted, Path::new("/home/gabe/.config/fish")));
        assert!(reachable(&granted, Path::new("/home/gabe/code/seal")));
        assert!(reachable(&granted, Path::new("/etc/hosts")));
    }

    #[test]
    fn a_nested_denial_does_not_shut_out_its_parent() {
        let granted = granted_for(&["/home/gabe/.config/gcloud"]);
        assert!(!reachable(&granted, Path::new("/home/gabe/.config/gcloud")));
        assert!(reachable(&granted, Path::new("/home/gabe/.config/fish")));
        assert!(reachable(&granted, Path::new("/home/gabe/code")));
    }

    #[test]
    fn another_users_home_is_still_reachable_unless_denied() {
        // Landlock is about this agent's credentials, not general isolation;
        // pretending otherwise would overstate what the jail does.
        let granted = granted_for(&["/home/gabe/.ssh"]);
        assert!(reachable(&granted, Path::new("/home/other")));
    }

    #[test]
    fn denying_nothing_grants_the_whole_filesystem() {
        let granted = allow_list(&[], &FakeTree);
        assert_eq!(granted, vec![PathBuf::from("/")]);
        assert!(reachable(&granted, Path::new("/anywhere/at/all")));
    }

    #[test]
    fn grants_are_not_redundant() {
        let granted = granted_for(&["/home/gabe/.ssh"]);
        for g in &granted {
            let covered_by_another = granted
                .iter()
                .any(|other| other != g && g.starts_with(other));
            assert!(!covered_by_another, "{} is already covered", g.display());
        }
    }

    #[test]
    fn a_path_that_does_not_exist_in_the_tree_is_simply_not_granted() {
        let granted = granted_for(&["/home/gabe/.ghost"]);
        assert!(!reachable(&granted, Path::new("/home/gabe/.ghost")));
        // Everything real is still there.
        assert!(reachable(&granted, Path::new("/home/gabe/code")));
        assert!(reachable(&granted, Path::new("/usr")));
    }

    #[test]
    fn the_profiles_denied_paths_produce_a_usable_grant_list() {
        // The real profile, against the fake tree.
        let denied: Vec<PathBuf> = [
            "/home/gabe/.ssh",
            "/home/gabe/.aws",
            "/home/gabe/.config/gcloud",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let granted = allow_list(&denied, &FakeTree);

        assert!(
            reachable(&granted, Path::new("/usr/bin/git")),
            "ordinary work must keep working"
        );
        assert!(!reachable(&granted, Path::new("/home/gabe/.ssh/config")));
    }

    #[test]
    fn landlock_is_reported_honestly_for_this_platform() {
        if cfg!(target_os = "linux") {
            // Kernels before 5.13 have none; either answer is valid.
            let _ = abi_version();
        } else {
            assert_eq!(abi_version(), None);
            assert!(apply(&[]).is_err(), "there is nothing to apply off Linux");
        }
    }
}
