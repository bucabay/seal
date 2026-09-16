//! Who is on the other end of the socket.
//!
//! Identity comes from the kernel, never from anything the caller sends. A
//! client can lie about every byte it writes; it cannot lie about the
//! credentials the kernel attaches to its connection.
//!
//! This is the reason the broker listens on a unix socket rather than stdio or
//! a TCP port: neither of those carries a peer identity at all.

use crate::error::{Error, Result};
use std::path::PathBuf;

/// The identity of a connected process, as the kernel reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerIdentity {
    pub uid: u32,
    pub gid: u32,
    pub pid: i32,
    /// Process start time, used to notice PID reuse. `None` where the platform
    /// will not tell us, in which case liveness is the weaker fallback.
    pub start_time: Option<u64>,
    /// Working directory at connect time, where it can be determined.
    pub cwd: Option<PathBuf>,
}

impl PeerIdentity {
    /// Whether this is still the same process, not merely the same number.
    ///
    /// A caller can connect, exit, and have its PID handed to something else
    /// before the broker finishes deciding. Comparing the start time closes
    /// that race wherever the platform exposes it.
    pub fn still_the_same_process(&self) -> bool {
        match self.start_time {
            Some(started) => process_start_time(self.pid) == Some(started),
            None => process_exists(self.pid),
        }
    }

    /// Reject a caller running as anybody else. The broker holds one user's
    /// secrets and has no business serving another.
    pub fn is_same_user_as_us(&self) -> bool {
        self.uid == current_uid()
    }
}

/// How the daemon learns who is calling. A trait so the dispatch logic can be
/// tested without opening sockets.
pub trait PeerSource: std::fmt::Debug {
    fn identify(&self) -> Result<PeerIdentity>;
}

#[cfg(unix)]
pub fn current_uid() -> u32 {
    // SAFETY: getuid cannot fail and touches no memory we own.
    unsafe { libc::getuid() as u32 }
}

#[cfg(not(unix))]
pub fn current_uid() -> u32 {
    0
}

#[cfg(unix)]
fn process_exists(pid: i32) -> bool {
    // SAFETY: signal 0 performs the permission and existence checks without
    // delivering anything.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn process_exists(_pid: i32) -> bool {
    false
}

/// Seconds-resolution process start time, where the platform provides it.
#[cfg(target_os = "macos")]
fn process_start_time(pid: i32) -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is exactly `size` bytes of writable memory, which is what
    // PROC_PIDTBSDINFO writes into. A short read is rejected below.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    if written != size {
        return None;
    }
    Some(info.pbi_start_tvsec)
}

#[cfg(target_os = "linux")]
fn process_start_time(pid: i32) -> Option<u64> {
    // Field 22 of /proc/<pid>/stat is starttime in clock ticks since boot.
    // The comm field may contain spaces and parentheses, so parse after the
    // final ')' rather than splitting the whole line.
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(19)?.parse::<u64>().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_start_time(_pid: i32) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn peer_cwd(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{}/cwd", pid)).ok()
}

#[cfg(target_os = "macos")]
fn peer_cwd(pid: i32) -> Option<PathBuf> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    // SAFETY: `info` is exactly `size` bytes of writable memory, which is what
    // PROC_PIDVNODEPATHINFO writes into. A short read is rejected below.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    if written != size {
        return None;
    }
    // `vip_path` is a fixed buffer declared as chunks; flatten it and stop at
    // the first NUL rather than trusting the whole array to be a path.
    let raw = info.pvi_cdir.vip_path;
    let bytes: Vec<u8> = raw
        .iter()
        .flat_map(|chunk| chunk.iter())
        .map(|&c| c as u8)
        .take_while(|&b| b != 0)
        .collect();
    if bytes.is_empty() {
        return None;
    }
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peer_cwd(_pid: i32) -> Option<PathBuf> {
    // Absent rather than guessed: policy that depends on the working directory
    // must check `cwd.is_some()` first.
    None
}

/// Read the peer's credentials off a connected unix socket.
#[cfg(unix)]
pub fn identify_stream(stream: &std::os::unix::net::UnixStream) -> Result<PeerIdentity> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();

    #[cfg(target_os = "linux")]
    let (uid, gid, pid) = {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` is exactly `len` bytes of writable memory and `fd` is
        // an open socket for the lifetime of the borrow.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc != 0 {
            return Err(Error::Os("SO_PEERCRED failed".into()));
        }
        (cred.uid, cred.gid, cred.pid)
    };

    #[cfg(target_os = "macos")]
    let (uid, gid, pid) = {
        let (mut uid, mut gid) = (0u32, 0u32);
        // SAFETY: both out-params are valid writable u32s.
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return Err(Error::Os("getpeereid failed".into()));
        }
        // LOCAL_PEERPID is a macOS extension; SOL_LOCAL is 0.
        const SOL_LOCAL: libc::c_int = 0;
        const LOCAL_PEERPID: libc::c_int = 0x002;
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        // SAFETY: as above; a failure here leaves `pid` at 0, handled below.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEERPID,
                &mut pid as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc != 0 {
            return Err(Error::Os("LOCAL_PEERPID failed".into()));
        }
        (uid, gid, pid)
    };

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let (uid, gid, pid) = {
        let (mut uid, mut gid) = (0u32, 0u32);
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return Err(Error::Os("getpeereid failed".into()));
        }
        (uid, gid, 0)
    };

    Ok(PeerIdentity {
        uid,
        gid,
        pid,
        start_time: process_start_time(pid),
        cwd: peer_cwd(pid),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_start_time_is_readable_and_stable() {
        let me = std::process::id() as i32;
        let a = process_start_time(me);
        assert!(
            a.is_some(),
            "this platform must expose a process start time"
        );
        assert_eq!(a, process_start_time(me), "start time must not drift");
    }

    #[test]
    fn a_live_process_is_detected_and_an_absent_one_is_not() {
        assert!(process_exists(std::process::id() as i32));
        // PID 0 is the kernel/swapper; kill(0, 0) addresses the process group,
        // so use an implausible PID instead.
        assert!(!process_exists(0x7FFF_FFF0));
    }

    #[test]
    fn identity_of_this_process_verifies_as_unchanged() {
        let me = PeerIdentity {
            uid: current_uid(),
            gid: 0,
            pid: std::process::id() as i32,
            start_time: process_start_time(std::process::id() as i32),
            cwd: None,
        };
        assert!(me.still_the_same_process());
        assert!(me.is_same_user_as_us());
    }

    #[test]
    fn a_recycled_pid_is_rejected() {
        // Same PID, a start time that does not match: exactly what PID reuse
        // looks like from the broker's side.
        let impostor = PeerIdentity {
            uid: current_uid(),
            gid: 0,
            pid: std::process::id() as i32,
            start_time: Some(1),
            cwd: None,
        };
        assert!(
            !impostor.still_the_same_process(),
            "a reused PID must not pass as the original caller"
        );
    }

    #[test]
    fn another_users_connection_is_refused() {
        let other = PeerIdentity {
            uid: current_uid().wrapping_add(1),
            gid: 0,
            pid: std::process::id() as i32,
            start_time: None,
            cwd: None,
        };
        assert!(!other.is_same_user_as_us());
    }

    #[test]
    fn a_dead_process_without_a_start_time_fails_liveness() {
        let gone = PeerIdentity {
            uid: current_uid(),
            gid: 0,
            pid: 0x7FFF_FFF0,
            start_time: None,
            cwd: None,
        };
        assert!(!gone.still_the_same_process());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn the_peer_working_directory_is_readable() {
        let me = std::process::id() as i32;
        let cwd = peer_cwd(me).expect("this platform must report a cwd");
        assert_eq!(
            cwd.canonicalize().ok(),
            std::env::current_dir()
                .ok()
                .and_then(|p| p.canonicalize().ok())
        );
    }

    #[cfg(unix)]
    #[test]
    fn credentials_come_off_a_real_socket_and_match_this_process() {
        use std::os::unix::net::UnixStream;
        let (a, _b) = UnixStream::pair().expect("socketpair");
        let id = identify_stream(&a).expect("identify");

        assert_eq!(id.uid, current_uid());
        assert_eq!(
            id.pid,
            std::process::id() as i32,
            "both ends of a socketpair are this process"
        );
        assert!(id.still_the_same_process());
        assert!(id.is_same_user_as_us());
    }
}
