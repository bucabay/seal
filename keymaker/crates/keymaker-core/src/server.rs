//! The unix-socket server and the thin client.
//!
//! The socket is the trust boundary. It lives in a directory only this user can
//! enter, and every connection is identified by the kernel before a single
//! request is read.
//!
//! The loop is single-threaded on purpose: the broker owns mutable state and a
//! local broker serves one agent at a time, so a queue is correct and a mutex
//! would only add a way to get it wrong.

use crate::broker::{Broker, Connection};
use crate::error::{Error, Result};
use crate::peer::{identify_stream, PeerIdentity};
use crate::protocol::{decode_request, decode_response, encode, Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Default socket location. Under the user's runtime/config directory, never
/// `/tmp`, where another user could race the path.
pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/keymaker/broker.sock")
    } else {
        PathBuf::from(home).join(".config/keymaker/broker.sock")
    }
}

#[cfg(unix)]
fn lock_down(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::Os(format!("creating {}: {}", dir.display(), e)))?;
    // 0700: nobody else may even enter the directory the socket sits in.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| Error::Os(format!("securing {}: {}", dir.display(), e)))
}

/// A bound socket, together with the lock that proves this process owns it.
///
/// The lock must outlive the listener: dropping it would let a second broker
/// decide the socket is stale and unlink it from under this one.
#[derive(Debug)]
pub struct BoundSocket {
    pub listener: UnixListener,
    path: PathBuf,
    /// Held open for the lifetime of the broker. `flock` is released when the
    /// descriptor closes, including if the process is killed.
    lock: std::fs::File,
}

impl BoundSocket {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        // Tidy up so the next start does not have to treat this as stale.
        let _ = std::fs::remove_file(&self.path);
        let _ = &self.lock;
    }
}

/// Take the advisory lock for `path`, or report who has it.
///
/// Probing with `connect()` is not good enough: a listener that has just closed
/// can still accept a connection for a moment, so "is anyone listening?" is a
/// race. `flock` is atomic and is released by the kernel even if the holder is
/// killed, which is exactly the question being asked.
#[cfg(unix)]
fn take_lock(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let lock_path = path.with_extension("lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| Error::Os(format!("opening {}: {}", lock_path.display(), e)))?;
    // SAFETY: the descriptor is open and owned by `file` for the call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return Err(Error::Os(format!(
            "a broker is already running on {}",
            path.display()
        )));
    }
    Ok(file)
}

/// The longest a unix socket path may be. The kernel copies it into a fixed
/// `sun_path` buffer — 104 bytes on macOS, 108 on Linux — and a path over the
/// limit fails with an error that says nothing useful about why.
pub const MAX_SOCKET_PATH: usize = 100;

/// Bind the listener, replacing a socket left behind by a dead broker.
pub fn bind(path: &Path) -> Result<BoundSocket> {
    let len = path.as_os_str().len();
    if len > MAX_SOCKET_PATH {
        return Err(Error::Os(format!(
            "socket path is {} bytes, limit is {}: {}",
            len,
            MAX_SOCKET_PATH,
            path.display()
        )));
    }
    if let Some(dir) = path.parent() {
        lock_down(dir)?;
    }

    // Holding the lock means no live broker owns this socket, so whatever file
    // is there belongs to one that died.
    let lock = take_lock(path)?;
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|e| Error::Os(format!("removing stale socket: {}", e)))?;
    }
    let listener = UnixListener::bind(path)
        .map_err(|e| Error::Os(format!("binding {}: {}", path.display(), e)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| Error::Os(format!("securing socket: {}", e)))?;
    }
    Ok(BoundSocket {
        listener,
        path: path.to_path_buf(),
        lock,
    })
}

/// Serve one connection to completion. Returns the number of requests handled.
pub fn serve_connection(broker: &mut Broker, stream: UnixStream) -> Result<usize> {
    let peer: PeerIdentity = identify_stream(&stream)?;

    // Refuse before reading a byte. A caller from another account has no
    // business here regardless of what it would have asked for.
    if !peer.is_same_user_as_us() {
        let mut out = stream;
        let _ = writeln!(
            out,
            "{}",
            encode(&Response::error("forbidden", "wrong user"))
        );
        return Ok(0);
    }

    let mut conn = Connection::new(peer);
    let reader = BufReader::new(stream.try_clone().map_err(|e| Error::Os(e.to_string()))?);
    let mut writer = stream;
    let mut handled = 0;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // client went away mid-message
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match decode_request(&line) {
            Ok(req) => broker.dispatch(&mut conn, req),
            // A malformed line is answered, not fatal: a confused client should
            // learn what it did wrong rather than be dropped.
            Err(e) => Response::error("parse", e),
        };
        if writeln!(writer, "{}", encode(&response)).is_err() {
            break;
        }
        handled += 1;
    }

    if let Some(session) = &conn.session {
        broker.close_session(session);
    }
    Ok(handled)
}

/// Accept connections until the process is stopped.
pub fn serve(broker: &mut Broker, bound: &BoundSocket) -> Result<()> {
    for stream in bound.listener.incoming() {
        match stream {
            Ok(s) => {
                // One bad connection must not stop the broker.
                let _ = serve_connection(broker, s);
            }
            Err(e) => return Err(Error::Os(format!("accept failed: {}", e))),
        }
    }
    Ok(())
}

/// The thin client. Carries a request; holds no key material.
#[derive(Debug)]
pub struct Client {
    pub(crate) reader: BufReader<UnixStream>,
    pub(crate) writer: UnixStream,
    pub session: Option<String>,
}

impl Client {
    pub fn connect(path: &Path) -> Result<Client> {
        let stream = UnixStream::connect(path).map_err(|e| {
            Error::Os(format!(
                "no broker at {} ({}). Start one with `keymaker serve`.",
                path.display(),
                e
            ))
        })?;
        let reader = BufReader::new(stream.try_clone().map_err(|e| Error::Os(e.to_string()))?);
        let mut client = Client {
            reader,
            writer: stream,
            session: None,
        };
        match client.send(Request::Hello {
            version: crate::protocol::PROTOCOL_VERSION,
        })? {
            Response::Hello { session, .. } => {
                client.session = Some(session);
                Ok(client)
            }
            Response::Error { message, .. } => {
                Err(Error::Os(format!("handshake refused: {}", message)))
            }
            other => Err(Error::Os(format!(
                "unexpected handshake reply: {:?}",
                other
            ))),
        }
    }

    pub fn send(&mut self, req: Request) -> Result<Response> {
        writeln!(self.writer, "{}", encode(&req))
            .map_err(|e| Error::Os(format!("writing to broker: {}", e)))?;
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(|e| Error::Os(format!("reading from broker: {}", e)))?;
        if n == 0 {
            return Err(Error::Os("broker closed the connection".into()));
        }
        decode_response(line.trim()).map_err(|e| Error::Parse(format!("bad reply: {}", e)))
    }

    /// Ask for a handle and spend it in one step, which is how every caller
    /// actually uses this.
    pub fn grant_and_run(&mut self, task: &str, env: &str) -> Result<Response> {
        let handle = match self.send(Request::Grant {
            capability: task.to_string(),
            kind: crate::protocol::GrantKind::Task,
        })? {
            Response::Granted { handle, .. } => handle,
            other => return Ok(other),
        };
        self.send(Request::RunTask {
            handle,
            env: env.to_string(),
        })
    }

    pub fn grant_and_call(
        &mut self,
        endpoint: &str,
        draft: crate::provider::RequestDraft,
    ) -> Result<Response> {
        let handle = match self.send(Request::Grant {
            capability: endpoint.to_string(),
            kind: crate::protocol::GrantKind::Request,
        })? {
            Response::Granted { handle, .. } => handle,
            other => return Ok(other),
        };
        self.send(Request::Call { handle, draft })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{HttpResponse, Transport};
    use crate::clock::FixedClock;
    use crate::id::SeqEntropy;
    use crate::manifest::Manifest;
    use crate::protocol::GrantKind;
    use crate::provider::{Catalog, PreparedRequest};
    use crate::runner::{RawOutcome, Spawner};
    use crate::store::MemoryStore;
    use std::collections::BTreeMap;

    #[derive(Debug)]
    struct Echo;
    impl Spawner for Echo {
        fn spawn(&self, command: &str, env: &BTreeMap<String, String>) -> Result<RawOutcome> {
            Ok(RawOutcome {
                exit_code: Some(0),
                stdout: format!("ran {} with {} vars\n", command, env.len()).into_bytes(),
                stderr: Vec::new(),
            })
        }
    }

    #[derive(Debug)]
    struct NoNetwork;
    impl Transport for NoNetwork {
        fn send(&self, _: &PreparedRequest) -> Result<HttpResponse> {
            Err(Error::Os("no network in tests".into()))
        }
    }

    const MANIFEST: &str = r#"
version = 1
[tasks]
deploy = "./deploy.sh"
[env.production]
DATABASE_URL = "hardroad/db_url"
"#;

    /// A socket path inside a fresh directory, cleaned up on drop.
    struct TempSocket(PathBuf);

    impl TempSocket {
        fn new(name: &str) -> TempSocket {
            // Kept deliberately short: the whole path must fit in `sun_path`,
            // and macOS temp directories are already long.
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("km{}-{}-{}", std::process::id(), n, name));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempSocket(dir.join("broker.sock"))
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempSocket {
        fn drop(&mut self) {
            if let Some(dir) = self.0.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    /// Run `body` against a live broker on a real socket.
    fn with_broker<F: FnOnce(&mut Client) + Send>(sock: &TempSocket, body: F) {
        let clock = FixedClock::new(1_000);
        let entropy = SeqEntropy::new();
        let store = MemoryStore::with([("hardroad/db_url", "postgres://u:hunter2@db/prod")]);
        let spawner = Echo;
        let transport = NoNetwork;
        let mut broker = Broker::new(
            &clock,
            &entropy,
            &store,
            &spawner,
            &transport,
            Manifest::from_toml(MANIFEST).unwrap(),
            Catalog::default(),
            60,
        );
        let bound = bind(sock.path()).expect("bind");

        // The broker stays on this thread: it owns `&mut` state and borrows a
        // clock and entropy source that are deliberately not `Sync`. The client
        // is the half that moves.
        std::thread::scope(|s| {
            let path = sock.path().to_path_buf();
            let client_thread = s.spawn(move || {
                let mut client = Client::connect(&path).expect("connect");
                body(&mut client);
                // Dropping the client here ends the server loop below.
            });
            let (stream, _) = bound.listener.accept().expect("accept");
            serve_connection(&mut broker, stream).expect("serve");
            client_thread.join().expect("client thread");
        });
    }

    #[test]
    fn a_client_handshakes_and_gets_a_session() {
        let sock = TempSocket::new("handshake");
        with_broker(&sock, |c| {
            assert!(c.session.is_some(), "handshake must yield a session");
        });
    }

    #[test]
    fn a_task_runs_over_the_socket_end_to_end() {
        let sock = TempSocket::new("run");
        with_broker(&sock, |c| {
            match c.grant_and_run("deploy", "production").unwrap() {
                Response::Ran {
                    exit_code, stdout, ..
                } => {
                    assert_eq!(exit_code, Some(0));
                    assert_eq!(stdout, "ran ./deploy.sh with 1 vars\n");
                }
                other => panic!("expected a run, got {:?}", other),
            }
        });
    }

    #[test]
    fn a_listing_over_the_socket_returns_names_only() {
        let sock = TempSocket::new("list");
        with_broker(&sock, |c| {
            match c
                .send(Request::ListRefs {
                    env: "production".into(),
                })
                .unwrap()
            {
                Response::Names { names } => {
                    assert_eq!(names, vec!["hardroad/db_url"]);
                    assert!(!format!("{:?}", names).contains("hunter2"));
                }
                other => panic!("expected names, got {:?}", other),
            }
        });
    }

    #[test]
    fn there_is_no_request_that_gets_a_value_back_over_the_wire() {
        let sock = TempSocket::new("noread");
        with_broker(&sock, |c| {
            // Hand-written wire messages, bypassing the typed client.
            for raw in [
                r#"{"op":"get","key":"hardroad/db_url"}"#,
                r#"{"op":"read","key":"hardroad/db_url"}"#,
                r#"{"op":"reveal","key":"hardroad/db_url"}"#,
            ] {
                writeln!(c.writer, "{}", raw).unwrap();
                let mut line = String::new();
                c.reader.read_line(&mut line).unwrap();
                let resp = decode_response(line.trim()).unwrap();
                assert!(resp.is_error(), "`{}` must be refused, got {:?}", raw, resp);
                assert!(!line.contains("hunter2"), "a value came back over the wire");
            }
        });
    }

    #[test]
    fn a_malformed_line_is_answered_rather_than_dropping_the_connection() {
        let sock = TempSocket::new("malformed");
        with_broker(&sock, |c| {
            writeln!(c.writer, "this is not json").unwrap();
            let mut line = String::new();
            c.reader.read_line(&mut line).unwrap();
            assert!(decode_response(line.trim()).unwrap().is_error());

            // The connection still works afterwards.
            match c.send(Request::ListTasks).unwrap() {
                Response::Names { names } => assert_eq!(names, vec!["deploy"]),
                other => panic!("connection did not survive: {:?}", other),
            }
        });
    }

    #[test]
    fn handles_do_not_survive_the_connection_that_earned_them() {
        let sock = TempSocket::new("lifetime");
        let clock = FixedClock::new(1_000);
        let entropy = SeqEntropy::new();
        let store = MemoryStore::with([("hardroad/db_url", "x")]);
        let spawner = Echo;
        let transport = NoNetwork;
        let mut broker = Broker::new(
            &clock,
            &entropy,
            &store,
            &spawner,
            &transport,
            Manifest::from_toml(MANIFEST).unwrap(),
            Catalog::default(),
            600,
        );
        let bound = bind(sock.path()).expect("bind");

        let stolen: String = std::thread::scope(|s| {
            let path = sock.path().to_path_buf();
            let client_thread = s.spawn(move || {
                let mut c = Client::connect(&path).unwrap();
                let handle = match c
                    .send(Request::Grant {
                        capability: "deploy".into(),
                        kind: GrantKind::Task,
                    })
                    .unwrap()
                {
                    Response::Granted { handle, .. } => handle,
                    other => panic!("expected a grant: {:?}", other),
                };
                drop(c); // the connection closes, so the session ends

                let mut c2 = Client::connect(&path).unwrap();
                let r = c2
                    .send(Request::RunTask {
                        handle: handle.clone(),
                        env: "production".into(),
                    })
                    .unwrap();
                assert!(
                    r.is_error(),
                    "a handle from a closed session must not work on a new one: {:?}",
                    r
                );
                drop(c2);
                handle
            });

            // Serve both connections in turn.
            for _ in 0..2 {
                let (stream, _) = bound.listener.accept().unwrap();
                serve_connection(&mut broker, stream).unwrap();
            }
            client_thread.join().unwrap()
        });
        assert_eq!(stolen.len(), 64);
    }

    // Socket ownership across restarts is covered by
    // tests/socket_ownership.rs, which runs in its own process: binding one
    // path repeatedly here interferes with the parallel socket tests above.

    #[test]
    fn the_socket_directory_is_private_to_this_user() {
        use std::os::unix::fs::PermissionsExt;
        let sock = TempSocket::new("perms");
        let _bound = bind(sock.path()).expect("bind");

        let dir = sock.path().parent().unwrap();
        let dir_mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            dir_mode, 0o700,
            "nobody else may enter the socket directory"
        );

        let sock_mode = std::fs::metadata(sock.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(sock_mode, 0o600, "nobody else may open the socket");
    }

    #[test]
    fn an_over_long_socket_path_is_refused_with_a_clear_reason() {
        let long = std::env::temp_dir().join("k".repeat(MAX_SOCKET_PATH + 1));
        let err = bind(&long).unwrap_err();
        let text = format!("{}", err);
        assert!(text.contains("socket path is"), "unhelpful error: {}", text);
        assert!(text.contains("limit is"));
    }

    #[test]
    fn the_default_socket_path_fits() {
        let p = default_socket_path();
        assert!(
            p.as_os_str().len() <= MAX_SOCKET_PATH,
            "the default path must be bindable: {} bytes",
            p.as_os_str().len()
        );
    }

    #[test]
    fn connecting_to_nothing_says_how_to_start_a_broker() {
        let sock = TempSocket::new("absent");
        let err = Client::connect(sock.path()).unwrap_err();
        assert!(format!("{}", err).contains("keymaker serve"));
    }
}
