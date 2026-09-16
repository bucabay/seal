//! Socket ownership across broker restarts.
//!
//! This lives in its own integration-test binary on purpose. It binds the same
//! path several times in one process, which is fine for a real broker (one
//! process binds once) but interferes with other socket tests when they share
//! a process and run in parallel. Isolating it keeps the assertion meaningful
//! rather than flaky.

use keymaker_core::server::bind;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let dir = std::env::temp_dir().join(format!("km-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
    fn sock(&self) -> PathBuf {
        self.0.join("broker.sock")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_socket_is_owned_by_one_broker_and_released_when_it_exits() {
    let dir = TempDir::new();
    let path = dir.sock();

    let first = bind(&path).expect("the first broker should bind");

    // While it is live, nobody else may take the socket.
    let second = bind(&path);
    let message = match second {
        Ok(_) => panic!("a second broker stole a live socket"),
        Err(e) => e.to_string(),
    };
    assert!(
        message.contains("already running"),
        "the refusal should say why: {}",
        message
    );

    // When it exits, the socket becomes available again. This is the case that
    // matters in practice: a broker that was killed must not lock everyone out.
    drop(first);
    bind(&path).expect("the socket should be free once its owner has gone");
}

#[test]
fn a_socket_file_left_by_a_crashed_broker_does_not_block_startup() {
    let dir = TempDir::new();
    let path = dir.0.join("crashed.sock");

    // A crash leaves the file on disk with no lock held.
    {
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("plain bind");
        std::mem::forget(listener);
    }
    assert!(path.exists());

    bind(&path).expect("a crashed broker's socket must be reclaimed, not fatal");
}
