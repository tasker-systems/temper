//! The exporter's HTTP client, observed: every send is timed, and a failure keeps its real cause.
//!
//! ## Why the client, and not the exporter
//!
//! `opentelemetry-otlp` 0.33.1 turns **every** client error into the string
//! `HTTP export failed: network error` (`OtlpHttpClient::export_http_once`). The reqwest error that
//! says *what* failed (a timeout, a refused connection, a peer that closed before answering) is
//! logged only at `debug` (`HttpClient.NetworkError`), and production runs at `info`. A wrapper
//! around the `SpanExporter` would see the flattened string and nothing more. Only the client still
//! holds the original error, so the client is where it is read.
//!
//! The same seam is the only place a send's duration exists. Nothing else measures it: the flush
//! budget's warning says a flush took over [`FLUSH_BUDGET`](super::FLUSH_BUDGET), not how long any
//! round trip actually was.
//!
//! ## What is logged, and at which level
//!
//! A line describes one **send attempt**, not one export. The SDK retries a failed export up to
//! three times (`RetryPolicy::recommended`, which treats a network error as retryable) and calls the
//! client once per attempt. So an export that fails outright logs up to four `span export failed`
//! lines before the SDK's one flattened `ERROR`, and an export that a retry rescues logs its failed
//! attempts with no SDK `ERROR` at all. The attempts of one export show as consecutive lines whose
//! `idle_ms` is the SDK's backoff, about 100 to 1,700 ms.
//!
//! - **A failed attempt: `warn`,** with the error chain, and `timeout` and `connect` from the first
//!   reqwest error in that chain.
//! - **An attempt that got a response but took at least `slow`: `info`.** In production `slow` is
//!   `FLUSH_BUDGET`, reused so that no new threshold is introduced. It is not the flush budget's
//!   measure: that one times a whole flush (queue wait, every attempt and the backoffs between
//!   them), and this one times a single attempt.
//! - **A faster attempt that got a response: nothing.** It is counted instead. Each logged line
//!   carries `quiet_sends`, the number of such attempts since the previous logged line, whatever
//!   their status, so the log alone gives the share of attempts that were slow or failed without a
//!   line per request. A non-2xx response is counted rather than logged because the SDK already
//!   reports it with its status code, which it does not flatten.
//!
//! Every line carries `send_ms` and `wall_ms`, the attempt's duration on the monotonic and the wall
//! clock, and `idle_ms` and `idle_wall_ms`, the same two clocks for the gap since the previous
//! attempt on this client finished. Whether a sandbox freeze stops the monotonic clock is not known.
//! Both clocks are logged so that a freeze shows either way: the two readings of a span that
//! includes one disagree if it stops the monotonic clock, and agree on a long gap if it does not. A
//! long idle before a slow or failed attempt points at a connection that went stale while frozen.
//!
//! ## What a log line never carries
//!
//! The collector URL. reqwest's `Display` for an error includes it (`… for url (…)`), so the error
//! is stripped with `reqwest::Error::without_url` before it is logged or returned. The endpoint's
//! host is already logged once at startup (`span export on`). The URL's path and query are the
//! operator's and are never echoed, the same rule [`vet_endpoint`](super::vet_endpoint) keeps.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};

/// The exporter's reqwest client, with each send timed and each failure's cause kept.
#[derive(Debug)]
pub(crate) struct ObservedClient {
    inner: reqwest::blocking::Client,
    /// An attempt with a response that takes at least this long is logged. `FLUSH_BUDGET` in
    /// production.
    slow: Duration,
    ledger: Mutex<Ledger>,
}

/// What a logged line needs from the attempts before it.
#[derive(Debug, Default)]
struct Ledger {
    /// When the previous attempt on this client finished, on both clocks.
    last_finished: Option<(Instant, SystemTime)>,
    /// Attempts with a response, under `slow`, since the last logged line.
    quiet_sends: u64,
}

impl ObservedClient {
    pub(crate) fn new(inner: reqwest::blocking::Client, slow: Duration) -> Self {
        Self {
            inner,
            slow,
            ledger: Mutex::new(Ledger::default()),
        }
    }

    /// Take the ledger's lock, recovering it if a panicking holder poisoned it. The ledger is
    /// diagnostics: a poisoned lock must never stop an export.
    fn ledger(&self) -> std::sync::MutexGuard<'_, Ledger> {
        self.ledger
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Milliseconds, saturating. A `u128` that overflows `u64` is not a duration anyone will log.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Wall-clock milliseconds since `since`. A wall clock that moved backwards reads as zero rather
/// than failing the export.
fn wall_millis(since: SystemTime) -> u64 {
    millis(since.elapsed().unwrap_or_default())
}

/// `error`, with the collector URL removed if it is a reqwest error.
///
/// Any other error keeps its original box. That matters beyond logging: the SDK downcasts the
/// returned error to `ResponseBodyTooLarge` to classify it as not retryable, and a re-boxed wrapper
/// would make it retryable.
fn without_url(error: HttpError) -> HttpError {
    match error.downcast::<reqwest::Error>() {
        Ok(reqwest_error) => Box::new(reqwest_error.without_url()),
        Err(other) => other,
    }
}

/// The error and its sources, joined with `": "`, outermost first.
fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut chain = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        chain.push_str(": ");
        chain.push_str(&cause.to_string());
        source = cause.source();
    }
    chain
}

/// The first reqwest error in `error`'s chain. A failure reading the response body arrives as a
/// `std::io::Error` wrapping reqwest's, so the top level alone would miss its `timeout`.
fn first_reqwest_error<'e>(
    error: &'e (dyn std::error::Error + 'static),
) -> Option<&'e reqwest::Error> {
    let mut current = Some(error);
    while let Some(candidate) = current {
        if let Some(reqwest_error) = candidate.downcast_ref::<reqwest::Error>() {
            return Some(reqwest_error);
        }
        current = candidate.source();
    }
    None
}

#[async_trait::async_trait]
impl HttpClient for ObservedClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let body_bytes = request.body().len();
        let (idle_ms, idle_wall_ms) = match self.ledger().last_finished {
            Some((at, at_wall)) => (Some(millis(at.elapsed())), Some(wall_millis(at_wall))),
            None => (None, None),
        };

        let started = Instant::now();
        let started_wall = SystemTime::now();
        let result = self.inner.send_bytes(request).await.map_err(without_url);
        let send_ms = millis(started.elapsed());
        let wall_ms = wall_millis(started_wall);

        let mut ledger = self.ledger();
        ledger.last_finished = Some((Instant::now(), SystemTime::now()));

        match &result {
            Ok(_) if Duration::from_millis(send_ms) < self.slow => {
                ledger.quiet_sends += 1;
            }
            Ok(response) => {
                let quiet_sends = std::mem::take(&mut ledger.quiet_sends);
                drop(ledger);
                tracing::info!(
                    status = response.status().as_u16(),
                    send_ms,
                    wall_ms,
                    idle_ms,
                    idle_wall_ms,
                    body_bytes,
                    quiet_sends,
                    "span export was slow"
                );
            }
            Err(error) => {
                let quiet_sends = std::mem::take(&mut ledger.quiet_sends);
                drop(ledger);
                let reqwest_error = first_reqwest_error(error.as_ref());
                tracing::warn!(
                    error = %error_chain(error.as_ref()),
                    timeout = reqwest_error.is_some_and(reqwest::Error::is_timeout),
                    connect = reqwest_error.is_some_and(reqwest::Error::is_connect),
                    send_ms,
                    wall_ms,
                    idle_ms,
                    idle_wall_ms,
                    body_bytes,
                    quiet_sends,
                    "span export failed"
                );
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// Captured JSON log lines.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Capture {
        type Writer = Capture;
        fn make_writer(&'w self) -> Self::Writer {
            self.clone()
        }
    }

    impl Capture {
        fn lines(&self) -> Vec<serde_json::Value> {
            let bytes = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            String::from_utf8(bytes)
                .expect("utf-8 log output")
                .lines()
                .map(|line| serde_json::from_str(line).expect("a JSON log line"))
                .collect()
        }
    }

    /// How each accepted connection is answered.
    #[derive(Clone, Copy)]
    enum Answer {
        /// `200` after this delay.
        Ok(Duration),
        /// Read the request, then close without a response.
        Hangup,
    }

    /// Serve `answers` in order, one per connection, on an ephemeral port.
    fn collector(answers: Vec<Answer>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("listener address").port();
        std::thread::spawn(move || {
            for answer in answers {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf);
                match answer {
                    Answer::Ok(delay) => {
                        std::thread::sleep(delay);
                        // `Connection: close`, so every send opens a new connection and the answers
                        // line up one per send.
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                    Answer::Hangup => {}
                }
            }
        });
        port
    }

    /// Run one send to completion. reqwest's blocking client does its I/O inside the first poll, so
    /// the future is ready on that poll; a pending one would be a change in that client, not a slow
    /// collector.
    fn send(client: &ObservedClient, url: &str) -> Result<Response<Bytes>, HttpError> {
        let request = Request::post(url)
            .body(Bytes::from_static(b"spans"))
            .expect("a valid request");
        let mut future = std::pin::pin!(client.send_bytes(request));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => {
                panic!("the blocking client's send was not ready on first poll")
            }
        }
    }

    /// The reqwest blocking client, built off any runtime, as `exporter_http_client` builds it.
    fn reqwest_client() -> reqwest::blocking::Client {
        std::thread::spawn(|| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("build a reqwest client")
        })
        .join()
        .expect("client-building thread")
    }

    use std::future::Future as _;

    /// A collector that hangs up must produce a `warn` line naming reqwest's cause, never the URL,
    /// and carrying the count of fast exports before it. On the SDK's own path this event is only
    /// `HTTP export failed: network error`.
    #[test]
    fn a_failed_send_logs_its_cause_without_the_url() {
        let port = collector(vec![Answer::Ok(Duration::ZERO), Answer::Hangup]);
        let url = format!("http://127.0.0.1:{port}/secret-path/v1/traces?token=abc");
        let client = ObservedClient::new(reqwest_client(), Duration::from_secs(60));
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(capture.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            assert!(
                send(&client, &url).is_ok(),
                "the first send is answered 200"
            );
            let error = send(&client, &url).expect_err("the second send is hung up on");
            assert!(
                !error.to_string().contains("secret-path"),
                "the returned error still carries the URL: {error}"
            );
        });

        let lines = capture.lines();
        assert_eq!(lines.len(), 1, "one line, for the failure only: {lines:#?}");
        let line = &lines[0];
        assert_eq!(line["level"], "WARN");
        let fields = &line["fields"];
        assert_eq!(fields["message"], "span export failed");
        let chain = fields["error"].as_str().expect("an error field");
        assert!(
            chain.starts_with("error sending request"),
            "the chain is reqwest's, not the SDK's flattened string: {chain}"
        );
        assert!(
            chain.contains(": "),
            "the chain includes at least one source beneath reqwest's own message: {chain}"
        );
        let rendered = line.to_string();
        assert!(
            !rendered.contains("secret-path") && !rendered.contains("token=abc"),
            "the log line echoes the collector URL: {rendered}"
        );
        assert_eq!(
            fields["quiet_sends"], 1,
            "the fast attempt before it is counted"
        );
        assert_eq!(fields["body_bytes"], 5);
        assert!(fields["send_ms"].is_u64() && fields["wall_ms"].is_u64());
        assert!(
            fields["idle_ms"].is_u64() && fields["idle_wall_ms"].is_u64(),
            "an attempt after a previous one reports the idle gap on both clocks: {fields}"
        );
    }

    /// A success at or over `slow` is logged at `info` with its timing, and the count of fast
    /// successes resets after each logged line.
    #[test]
    fn a_slow_success_is_logged_and_a_fast_one_is_counted() {
        let slow = Duration::from_millis(200);
        let port = collector(vec![
            Answer::Ok(Duration::ZERO),
            Answer::Ok(Duration::ZERO),
            Answer::Ok(Duration::from_millis(500)),
            Answer::Ok(Duration::from_millis(500)),
        ]);
        let url = format!("http://127.0.0.1:{port}/v1/traces");
        let client = ObservedClient::new(reqwest_client(), slow);
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(capture.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..4 {
                send(&client, &url).expect("every send is answered 200");
            }
        });

        let lines = capture.lines();
        assert_eq!(
            lines.len(),
            2,
            "the two slow sends, and only those: {lines:#?}"
        );
        for line in &lines {
            assert_eq!(line["level"], "INFO");
            assert_eq!(line["fields"]["message"], "span export was slow");
            assert_eq!(line["fields"]["status"], 200);
            let send_ms = line["fields"]["send_ms"].as_u64().expect("send_ms");
            assert!(
                send_ms >= 200,
                "a slow line reports a slow attempt: {send_ms}ms"
            );
        }
        assert_eq!(lines[0]["fields"]["quiet_sends"], 2);
        assert_eq!(
            lines[1]["fields"]["quiet_sends"], 0,
            "the count resets after the line that reported it"
        );
        assert!(
            lines[0]["fields"]["idle_ms"].is_u64(),
            "the third send follows the second, so it has an idle gap"
        );
    }

    /// The first send on a client has no previous send, so it reports no idle gap rather than a
    /// made-up one.
    #[test]
    fn the_first_send_reports_no_idle_gap() {
        let port = collector(vec![Answer::Hangup]);
        let url = format!("http://127.0.0.1:{port}/v1/traces");
        let client = ObservedClient::new(reqwest_client(), Duration::from_secs(60));
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(capture.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let _ = send(&client, &url);
        });

        let lines = capture.lines();
        assert_eq!(lines.len(), 1);
        assert!(
            ["idle_ms", "idle_wall_ms"]
                .iter()
                .all(|field| lines[0]["fields"]
                    .get(field)
                    .is_none_or(serde_json::Value::is_null)),
            "no previous send, so no idle gap: {}",
            lines[0]
        );
    }

    /// A non-reqwest error leaves `without_url` in its original box. The SDK downcasts the error it
    /// gets back to `ResponseBodyTooLarge` to decide it is not retryable; a wrapper would hide it.
    #[test]
    fn a_non_reqwest_error_keeps_its_type() {
        let error: HttpError = Box::new(opentelemetry_http::ResponseBodyTooLarge::new());
        let passed = without_url(error);
        assert!(
            passed
                .downcast_ref::<opentelemetry_http::ResponseBodyTooLarge>()
                .is_some(),
            "ResponseBodyTooLarge did not survive without_url: {passed}"
        );
    }
}
