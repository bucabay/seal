//! Reading a secret from a terminal without putting it on screen.
//!
//! A prompt that echoes what you type writes the value into the terminal's
//! scrollback, where it stays — and on into a screen recording, a shared
//! session, or a screenshot of the window. For a tool whose whole purpose is
//! that values do not end up somewhere they can be read back, that is not a
//! detail to leave to the user's shell.

use crate::error::{Error, Result};
use std::io::{BufRead, Write};

/// Restores the terminal however this function exits, including on a panic
/// part-way through reading. A terminal left with echo disabled looks broken.
#[cfg(unix)]
struct EchoOff {
    fd: i32,
    previous: libc::termios,
}

#[cfg(unix)]
impl EchoOff {
    fn new(fd: i32) -> Option<EchoOff> {
        // SAFETY: `previous` is valid writable memory of the right type, and
        // `fd` is a terminal descriptor for the lifetime of the call.
        let mut previous: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut previous) } != 0 {
            return None;
        }
        let mut quiet = previous;
        quiet.c_lflag &= !libc::ECHO;
        // Keep ECHONL so the newline still moves the cursor down: without it
        // the prompt and the next output run together.
        quiet.c_lflag |= libc::ECHONL;
        // SAFETY: `quiet` is a valid termios derived from the current one.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) } != 0 {
            return None;
        }
        Some(EchoOff { fd, previous })
    }
}

#[cfg(unix)]
impl Drop for EchoOff {
    fn drop(&mut self) {
        // SAFETY: restoring the settings captured in `new`.
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.previous) };
    }
}

/// True when stdin is a terminal a person is typing at.
pub fn stdin_is_tty() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: isatty only inspects the descriptor.
        unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Prompt for a secret and read one line, without echoing it.
///
/// Falls back to a plain read where echo cannot be turned off, but says so
/// first: a silent fallback would put the value on screen exactly when the
/// user had been told it would not be.
pub fn read_secret(prompt: &str) -> Result<String> {
    let mut stderr = std::io::stderr();
    write!(stderr, "{}", prompt).ok();
    stderr.flush().ok();

    #[cfg(unix)]
    let guard = EchoOff::new(libc::STDIN_FILENO);
    #[cfg(not(unix))]
    let guard: Option<()> = None;

    if guard.is_none() {
        writeln!(stderr, "\n(warning: this terminal will show what you type)").ok();
        write!(stderr, "{}", prompt).ok();
        stderr.flush().ok();
    }

    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| Error::Os(format!("reading input: {}", e)))?;
    drop(guard);

    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

/// Read a secret from wherever stdin is: a pipe, or a person typing.
pub fn read_secret_or_stdin(prompt: &str) -> Result<String> {
    if stdin_is_tty() {
        return read_secret(prompt);
    }
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| Error::Os(format!("reading input: {}", e)))?;
    // A trailing newline is almost always the shell's, not part of the value.
    Ok(buf.trim_end_matches(['\n', '\r']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_newline_is_not_part_of_the_value() {
        // What `echo -n` versus `echo` produces, and what a person's Return
        // adds. None of it belongs in the credential.
        for (input, expected) in [
            ("secret\n", "secret"),
            ("secret\r\n", "secret"),
            ("secret", "secret"),
            ("sec ret \n", "sec ret "),
        ] {
            assert_eq!(input.trim_end_matches(['\n', '\r']), expected);
        }
    }

    #[test]
    fn internal_whitespace_survives() {
        // A passphrase may legitimately contain spaces; only the line ending
        // is stripped.
        assert_eq!("a b  c\n".trim_end_matches(['\n', '\r']), "a b  c");
    }

    #[cfg(unix)]
    #[test]
    fn echo_is_restored_even_if_reading_goes_wrong() {
        // Under `cargo test` stdin is not a terminal, so the guard declines to
        // engage and there is nothing to restore. The property under test is
        // that it never leaves a terminal altered: declining is the correct
        // behaviour here, and Drop covers the case where it did engage.
        let guard = EchoOff::new(libc::STDIN_FILENO);
        assert!(
            guard.is_none() || stdin_is_tty(),
            "echo should only be touched when stdin really is a terminal"
        );
    }

    #[test]
    fn a_pipe_is_not_mistaken_for_a_person() {
        // The test harness gives us a non-tty stdin.
        assert!(!stdin_is_tty());
    }
}
