//! What this client is willing to put a secret on, checked once, at the seam.
//!
//! Two URLs arrive from configuration and then carry credentials on every use:
//! the API base URL — every request puts the bearer token on it — and the OAuth
//! token URL, where every refresh puts the refresh token on the wire and every
//! code exchange the authorization code and verifier. For both, the scheme is
//! not cosmetic: plaintext `http` off the loopback interface puts the credential
//! in the clear, readable by anything on the path.
//!
//! Checked at CONSTRUCTION, not at first use. A URL validated when a request is
//! built would surface its error several layers and possibly several minutes
//! from the configuration that caused it — the same discipline the Python
//! client states in `temper/_validate.py`.
//!
//! The deliberate escape hatch is [`allow_insecure_http_from_env`], for the case
//! this check cannot see: a private network where TLS terminates elsewhere. It is
//! a variable an operator has to set, which is the point — it must not be a typo
//! away.
//!
//! **Sibling parity, stated so nobody hunts for exactness that was never the
//! design.** The four clients' validators are not byte-identical on exotic
//! hosts — WHATWG parsers (this crate, temper-ts) normalize integer-IP forms
//! like `http://2130706433` to `127.0.0.1` and accept them as loopback, where
//! temper-py and the gem's RFC-3986 parsers refuse; a dotted trailing dot and
//! IPv4-mapped loopback split similarly across the four. Every disagreement
//! fails closed or is consistent with the parser the transport itself uses
//! (the URL that connects is the URL that validated). The invariant is the
//! wire, not the spelling.

use url::Url;

use crate::error::{ClientError, Result};

/// Hostnames that are the local machine by definition. `.localhost` is reserved
/// for exactly this by RFC 6761 §6.3, and Docker/CI setups do use `foo.localhost`.
const LOOPBACK_NAMES: [&str; 1] = ["localhost"];

/// Validate an endpoint this client is about to put a secret on.
///
/// * absolute `http`/`https`, with a host
/// * no userinfo — `https://id:secret@host/` would ride the secret in every
///   error message that names the URL
/// * no query or fragment — the client joins the base URL with request paths,
///   which would bury them mid-URL
/// * `http` only to the loopback interface, unless `allow_insecure_http`
pub fn validate_endpoint(value: &str, name: &str, allow_insecure_http: bool) -> Result<()> {
    fn not_configured(name: &str, what: &str) -> ClientError {
        ClientError::NotConfigured(format!("{name} {what}"))
    }

    // The WHATWG parser silently strips tab, CR and LF anywhere in the URL
    // (CVE-2019-9740's fix). Silently is the problem: a URL with an embedded
    // newline would be accepted here and normalized into something the caller
    // never wrote. Whitespace anywhere is refused rather than normalized for
    // the same reason.
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(not_configured(
            name,
            "must not contain whitespace or control characters",
        ));
    }

    let parsed = Url::parse(value)
        .map_err(|_| not_configured(name, &format!("is not a parseable URL: {value:?}")))?;
    // An out-of-range or non-numeric port fails `Url::parse` itself, so an
    // explicit port check here would have nothing left to catch.
    let scheme = parsed.scheme();
    let host = parsed.host_str();
    if !matches!(scheme, "http" | "https") || host.is_none() {
        return Err(not_configured(
            name,
            &format!("must be an absolute http(s) URL, got {value:?}"),
        ));
    }
    let host = host.expect("host checked above");

    // `username()` is empty for `host:port`, so this catches only a real
    // userinfo section. Refused rather than dropped: a caller who wrote
    // credentials into the URL meant them to authenticate something, and
    // quietly discarding them would produce a 401 whose cause is invisible.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(not_configured(
            name,
            "must not carry userinfo (user:password@); pass credentials through \
             the token store rather than the URL",
        ));
    }

    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(not_configured(
            name,
            "must be an origin (optionally with a path prefix), not a URL with \
             a query or fragment",
        ));
    }

    if scheme == "http" && !(allow_insecure_http || is_loopback(host)) {
        return Err(ClientError::NotConfigured(format!(
            "{name} is plaintext http to a non-loopback host, which would put the \
             bearer token and client_secret on the wire in the clear; use https, \
             or set TEMPER_ALLOW_INSECURE_HTTP=1 to accept that deliberately"
        )));
    }

    Ok(())
}

/// Whether `host` names this machine, by literal address or by reserved name.
///
/// [`Url::host_str`] is already lowercased, and any brackets around an IPv6
/// literal are stripped defensively here so a [`std::net::IpAddr`] parse sees
/// the bare address — it covers the whole `127.0.0.0/8` block, not just
/// `127.0.0.1`.
pub fn is_loopback(host: &str) -> bool {
    // One fully-qualified trailing dot is the same name — stripped before the
    // IP parse too, so `is_loopback("127.0.0.1.")` agrees with what
    // `validate_endpoint` accepts for the same host (the WHATWG parser
    // normalizes the dot away before `host_str` ever sees it).
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    LOOPBACK_NAMES.contains(&host.as_str()) || host.ends_with(".localhost")
}

/// The operator's deliberate opt-out for plaintext off the loopback.
///
/// `TEMPER_ALLOW_INSECURE_HTTP=1` (or `=true`) accepts a non-loopback `http`
/// endpoint for the case the scheme check cannot see: a private network where
/// TLS terminates elsewhere. An environment variable, sibling of
/// `TEMPER_API_URL`, the established override surface for endpoint-shaped
/// config. Read at the seam, never per request.
pub fn allow_insecure_http_from_env() -> bool {
    std::env::var("TEMPER_ALLOW_INSECURE_HTTP")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(url: &str) {
        validate_endpoint(url, "base_url", false)
            .unwrap_or_else(|e| panic!("should validate {url}: {e}"));
    }

    fn refused(url: &str) {
        let err = validate_endpoint(url, "base_url", false)
            .err()
            .unwrap_or_else(|| panic!("should refuse {url}"));
        assert!(
            matches!(err, ClientError::NotConfigured(_)),
            "expected NotConfigured for {url}, got {err:?}"
        );
    }

    #[test]
    fn accepts_an_https_origin_with_a_path_prefix() {
        ok("https://temperkb.io");
        ok("https://temperkb.io/api");
        ok("http://127.0.0.1:8080");
        ok("http://[::1]:3000");
        ok("http://app.localhost:9966");
    }

    #[test]
    fn rejects_anything_that_is_not_an_absolute_http_s_url() {
        refused("");
        refused("temperkb.io");
        refused("/api/relative");
        refused("ftp://temperkb.io");
        refused("http://");
    }

    #[test]
    fn rejects_userinfo_because_it_would_ride_in_every_error_message() {
        refused("https://id:secret@temperkb.io");
        refused("http://user@127.0.0.1");
    }

    #[test]
    fn rejects_a_query_or_fragment_that_the_path_join_would_bury() {
        refused("https://temperkb.io?audience=x");
        refused("https://temperkb.io#section");
    }

    #[test]
    fn rejects_an_embedded_newline_rather_than_letting_the_parser_strip_it() {
        refused("https://temperkb.io/\r\nx-auth");
        refused("https://temper\tkb.io");
    }

    #[test]
    fn rejects_an_unparseable_port() {
        refused("https://temperkb.io:99999");
        refused("https://temperkb.io:not-a-port");
    }

    #[test]
    fn allows_plaintext_to_the_loopback_interface() {
        ok("http://localhost");
        ok("http://localhost:3000");
        ok("http://127.0.0.1");
        ok("http://127.255.42.42:8123"); // the whole 127.0.0.0/8 block, not just .0.0.1
        ok("http://[::1]:0");
        ok("http://worker.localhost");
    }

    #[test]
    fn refuses_plaintext_to_anything_else() {
        refused("http://temperkb.io");
        refused("http://10.0.0.5:8080"); // private, but not loopback — not this check's call
        refused("http://192.168.1.10");
    }

    #[test]
    fn the_opt_out_is_a_variable_the_operator_has_to_set() {
        validate_endpoint("http://temperkb.io", "base_url", true)
            .expect("explicit opt-in accepts plaintext");
    }

    #[test]
    fn names_this_machine_by_literal_address_or_reserved_name() {
        assert!(is_loopback("localhost"));
        assert!(is_loopback("LOCALHOST")); // host_str is already lowercased; direct calls are not
        assert!(is_loopback("localhost.")); // one fully-qualified trailing dot
        assert!(is_loopback("app.localhost"));
        assert!(is_loopback("127.0.0.1"));
        assert!(is_loopback("127.9.9.9"));
        assert!(is_loopback("127.0.0.1.")); // trailing dot stripped before the IP parse, matching what the parser normalizes
        assert!(is_loopback("::1"));
        assert!(is_loopback("[::1]")); // brackets stripped for direct callers
        assert!(!is_loopback("temperkb.io"));
        assert!(!is_loopback("10.0.0.1"));
        assert!(!is_loopback("localhost.example.com")); // .localhost as a SUFFIX of a longer name
        assert!(!is_loopback("127.0.0.2.example.com"));
    }

    #[test]
    fn the_env_opt_out_reads_one_and_true() {
        temp_env::with_var("TEMPER_ALLOW_INSECURE_HTTP", Some("1"), || {
            assert!(allow_insecure_http_from_env());
        });
        temp_env::with_var("TEMPER_ALLOW_INSECURE_HTTP", Some("true"), || {
            assert!(allow_insecure_http_from_env());
        });
        temp_env::with_var("TEMPER_ALLOW_INSECURE_HTTP", Some("TRUE"), || {
            assert!(allow_insecure_http_from_env());
        });
        temp_env::with_var("TEMPER_ALLOW_INSECURE_HTTP", Some("0"), || {
            assert!(!allow_insecure_http_from_env());
        });
        temp_env::with_var("TEMPER_ALLOW_INSECURE_HTTP", None::<&str>, || {
            assert!(!allow_insecure_http_from_env());
        });
    }
}
