"""What a caller is allowed to hand this package.

Every case here is a value that reaches the wire — an origin a bearer token is sent
to, a string that becomes a header — so the assertions are about what leaves the
process, not about tidiness.
"""

from __future__ import annotations

import pytest

from temper._validate import is_loopback, require_endpoint, require_opaque, require_str


class TestRequireStr:
    @pytest.mark.parametrize("bad", ["", None, 123, b"bytes"])
    def test_rejects_anything_that_is_not_a_non_empty_str(self, bad):
        with pytest.raises(ValueError, match="non-empty str"):
            require_str(bad, "thing")


class TestRequireOpaque:
    def test_passes_an_ordinary_token_through_unchanged(self):
        assert require_opaque("eyJhbGciOiJSUzI1NiJ9.abc", "token") == "eyJhbGciOiJSUzI1NiJ9.abc"

    @pytest.mark.parametrize("bad", ["tok\n", "tok ", " tok", "\ttok"])
    def test_rejects_surrounding_whitespace_rather_than_stripping_it(self, bad):
        # `SECRET=$(cat secret.txt)` keeps the trailing newline. Stripping would be a
        # guess, and the same guess is wrong for a space in the middle of a secret.
        with pytest.raises(ValueError, match="whitespace"):
            require_opaque(bad, "token")

    def test_rejects_interior_whitespace(self):
        with pytest.raises(ValueError, match="whitespace"):
            require_opaque("two words", "token")

    def test_rejects_a_header_split(self):
        # `Authorization: Bearer <this>` — a CRLF here is a second header.
        with pytest.raises(ValueError, match="whitespace"):
            require_opaque("tok\r\nX-Admin: 1", "token")

    def test_rejects_a_control_character(self):
        with pytest.raises(ValueError, match="control characters"):
            require_opaque("tok\x00suffix", "token")


class TestRequireEndpoint:
    def test_accepts_an_https_origin_with_a_path_prefix(self):
        # The generated core joins host + `/api/...`, so an instance mounted under a
        # prefix is addressed by keeping the prefix on the host.
        parts = require_endpoint("https://temperkb.io/temper", name="base_url")
        assert parts.hostname == "temperkb.io"

    @pytest.mark.parametrize(
        "bad",
        ["", "temperkb.io", "ftp://temperkb.io", "https://", None, 7],
    )
    def test_rejects_anything_that_is_not_an_absolute_http_s_url(self, bad):
        with pytest.raises(ValueError):
            require_endpoint(bad, name="base_url")

    def test_rejects_userinfo_because_it_would_ride_in_every_error_message(self):
        # `urlsplit` accepts this happily, and the secret then appears in every
        # TransportError that names the URL.
        with pytest.raises(ValueError, match="userinfo"):
            require_endpoint("https://id:s3cret@auth.test/oauth/token", name="token_url")

    @pytest.mark.parametrize("bad", ["https://temperkb.io/?token=x", "https://temperkb.io/#frag"])
    def test_rejects_a_query_or_fragment_that_the_path_join_would_bury(self, bad):
        with pytest.raises(ValueError, match="query or fragment"):
            require_endpoint(bad, name="base_url")

    def test_rejects_an_embedded_newline_rather_than_letting_urlsplit_strip_it(self):
        # `urlsplit` silently drops CR/LF/tab, so this would otherwise be accepted as
        # a URL the caller never wrote.
        with pytest.raises(ValueError, match="control characters"):
            require_endpoint("https://temperkb.io\n/evil", name="base_url")

    def test_rejects_an_unparseable_port(self):
        with pytest.raises(ValueError):
            require_endpoint("https://temperkb.io:notaport", name="base_url")


class TestPlaintextHttp:
    @pytest.mark.parametrize(
        "url",
        [
            "http://127.0.0.1:8080",
            "http://127.0.0.5:8080",
            "http://localhost:3000",
            "http://[::1]:3000",
            "http://api.localhost",
        ],
    )
    def test_allows_plaintext_to_the_loopback_interface(self, url):
        # A test server and a `temper serve` on your laptop are both this.
        assert require_endpoint(url, name="base_url").scheme == "http"

    def test_refuses_plaintext_to_anything_else(self):
        with pytest.raises(ValueError, match="plaintext http"):
            require_endpoint("http://temperkb.io", name="base_url")

    def test_the_opt_out_is_a_keyword_the_caller_has_to_write(self):
        parts = require_endpoint(
            "http://temper.internal", name="base_url", allow_insecure_http=True
        )
        assert parts.hostname == "temper.internal"


class TestIsLoopback:
    @pytest.mark.parametrize(
        ("hostname", "expected"),
        [
            ("127.0.0.1", True),
            # The whole 127.0.0.0/8 block, not just .1.
            ("127.13.9.2", True),
            ("::1", True),
            ("localhost", True),
            ("api.localhost", True),
            ("temperkb.io", False),
            # Not loopback, and a classic almost-loopback.
            ("127.0.0.1.evil.test", False),
            ("0.0.0.0", False),
            (None, False),
            ("", False),
        ],
    )
    def test_names_this_machine_by_literal_address_or_reserved_name(self, hostname, expected):
        assert is_loopback(hostname) is expected
