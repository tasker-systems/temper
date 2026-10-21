//! A loopback listener that counts the connections made to it — the witness for "this compile
//! fetched nothing".
//!
//! A schema whose `$ref` names [`FetchProbe::url`] makes any fetching compiler connect here. The
//! count is read from the kernel's accept queue (the listener is non-blocking and drained on
//! each read), so a connection counts the moment its handshake completes, with no accept thread
//! to race the assertion. Also included, by path, from `temper-services`' reconcile test and the
//! e2e door tests: the suites must agree on what a connection is.

// `allow`, not `expect` (code-quality-best-practices §1.7, the cfg-conditional exception): this file
// is compiled into several test binaries, each using a different subset of `FetchProbe` (the A1
// witnesses use all of it; a door test reads only `url`, `port` and `connections`; a binary that
// pulls it in through `common` may use none), so the lint fires in some builds and not others and
// an `expect` would be unfulfilled in whichever binary uses everything.
#![allow(dead_code)]

use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
pub struct FetchProbe {
    listener: TcpListener,
    port: u16,
    seen: AtomicUsize,
}

impl FetchProbe {
    /// Bind `127.0.0.1:0` and start counting.
    pub fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback probe");
        listener
            .set_nonblocking(true)
            .expect("probe listener non-blocking");
        let port = listener.local_addr().expect("probe address").port();
        Self {
            listener,
            port,
            seen: AtomicUsize::new(0),
        }
    }

    /// An `http` URI on this listener, for a `$ref` or `$schema` to name.
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/{path}", self.port)
    }

    /// The port, for asserting an error text does not carry it.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Connections made to the listener so far.
    pub fn connections(&self) -> usize {
        while self.listener.accept().is_ok() {
            self.seen.fetch_add(1, Ordering::SeqCst);
        }
        self.seen.load(Ordering::SeqCst)
    }
}
