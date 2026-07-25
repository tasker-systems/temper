//! A one-shot OTLP-shaped HTTP sink, shared by the tests that drive the **real** init seams.
//!
//! ## Why these tests exist as separate binaries
//!
//! Each one installs a process-global `tracing` subscriber and a process-global tracer `PROVIDER`
//! (a `OnceLock`). nextest gives every test its own process, but `cargo test` shares a process across
//! the tests *within* one binary — so a suite that passes under the runner this repo uses and fails
//! under the one someone reaches for by hand is worse than a few extra files. One test per binary is
//! safe under both.
//!
//! ## Why a real socket rather than `InMemorySpanExporter`
//!
//! Two reasons, both learned expensively:
//!
//! 1. An in-memory exporter needs no HTTP client and no runtime, so it cannot observe a transport
//!    failure — that is how PR #535 shipped an exporter that could not reach any endpoint.
//! 2. It cannot be reached through the **real** init seam at all. `init_server_logging` calls
//!    `export_layer()`, which builds its own provider from the OTLP environment; a test that installs a
//!    different provider is testing a replica. An adversarial review proved that point by reverting two
//!    of this crate's fixes and watching all 31 tests stay green, because the only tests that covered
//!    them hand-built a copy of the stack instead of calling it.
//!
//! So: set the OTLP environment, call the public `init_*` function, and read bytes off a socket.
//!
//! Shared across several binaries, each using a different subset — unused helpers here are expected.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

/// How long to wait for an export before concluding none is coming.
///
/// Generous: the assertion "nothing was exported" must not be satisfiable by being impatient.
pub const EXPORT_WAIT: Duration = Duration::from_secs(10);

/// Bound on how long a *negative* case waits. Shorter by necessity — there is nothing to wait for —
/// but long enough that a slow machine does not manufacture a pass.
pub const NO_EXPORT_WAIT: Duration = Duration::from_secs(3);

/// A listener that accepts requests and reports each one's first bytes.
pub struct Sink {
    pub port: u16,
    pub requests: mpsc::Receiver<String>,
}

impl Sink {
    /// Bind an ephemeral port and serve until the test ends.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener
            .local_addr()
            .expect("listener has an address")
            .port();
        let (tx, requests) = mpsc::channel();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 8192];
                let read = stream.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..read]).to_string();
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\n\r\n",
                );
                let _ = stream.flush();
                if tx.send(head).is_err() {
                    return;
                }
            }
        });

        Self { port, requests }
    }

    /// A listener that accepts and then never answers.
    ///
    /// Models a degraded vendor deterministically. An unroutable address would depend on the host's
    /// network topology — a container may refuse instantly where a laptop black-holes — and a test
    /// about *timeouts* must not be at the mercy of how fast the failure arrives.
    ///
    /// The accepted stream is deliberately leaked into a parked thread: dropping it would send a FIN
    /// and let the client fail fast, which is the opposite of what is being modelled.
    pub fn start_silent() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener
            .local_addr()
            .expect("listener has an address")
            .port();
        let (tx, requests) = mpsc::channel();

        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = tx.send("<silent>".to_string());
                held.push(stream);
            }
        });

        Self { port, requests }
    }

    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Wait for one request, or panic with `context`.
    pub fn expect_request(&self, context: &str) -> String {
        self.requests
            .recv_timeout(EXPORT_WAIT)
            .unwrap_or_else(|e| panic!("{context} (waited {EXPORT_WAIT:?}, {e})"))
    }

    /// Assert that nothing arrives within [`NO_EXPORT_WAIT`].
    pub fn expect_silence(&self, context: &str) {
        match self.requests.recv_timeout(NO_EXPORT_WAIT) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(req) => panic!("{context} — but a request arrived: {req:?}"),
            Err(e) => panic!("{context} — sink channel died: {e}"),
        }
    }
}
