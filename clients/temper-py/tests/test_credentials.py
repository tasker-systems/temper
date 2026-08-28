import json
import threading
import time
from typing import Any

import pytest

from temper import BearerToken, ClientCredentials, TransportError, Unauthorized
from temper.credentials import MAX_TOKEN_RESPONSE_BYTES


class TestBearerToken:
    def test_returns_the_token_it_was_constructed_with_with_no_io(self):
        assert BearerToken("abc").token() == "abc"

    @pytest.mark.parametrize("bad", ["", None, 123])
    def test_rejects_a_non_string_or_empty_token_at_construction(self, bad):
        with pytest.raises(ValueError):
            BearerToken(bad)

    def test_cannot_refresh_and_says_so(self):
        creds = BearerToken("abc")
        assert creds.can_refresh is False
        with pytest.raises(Unauthorized):
            creds.refresh()


class Clock:
    def __init__(self, now: float = 1_000_000.0) -> None:
        self.now = now

    def __call__(self) -> float:
        return self.now


def credentials(server, clock=None, audience="https://api.test"):
    return ClientCredentials(
        token_url=f"{server.url}/oauth/token",
        client_id="cid",
        client_secret="sec",
        audience=audience,
        clock=clock or Clock(),
    )


def mint(server, token: str, expires_in: int = 3600, times: int = 1):
    server.respond_json({"access_token": token, "expires_in": expires_in}, times=times)


class TestClientCredentials:
    @pytest.mark.parametrize(
        "kwargs",
        [
            {"token_url": ""},
            {"client_id": ""},
            {"client_secret": ""},
            {"audience": ""},
        ],
    )
    def test_rejects_empty_required_values_at_construction(self, kwargs):
        base: dict[str, Any] = {
            "token_url": "https://auth.test/oauth/token",
            "client_id": "cid",
            "client_secret": "sec",
        }
        base.update(kwargs)
        with pytest.raises(ValueError):
            ClientCredentials(**base)

    def test_mints_on_first_use_with_the_m2m_fields(self, server):
        mint(server, "tok-1")
        assert credentials(server).token() == "tok-1"

        assert server.requests[-1].form() == {
            "grant_type": "client_credentials",
            "client_id": "cid",
            "client_secret": "sec",
            "audience": "https://api.test",
        }

    def test_omits_an_audience_it_was_not_given(self, server):
        # temper's own AS ignores a request-supplied audience and mints with its
        # server-side AS_AUDIENCE. Sending an empty one would be a lie.
        mint(server, "tok-1")
        credentials(server, audience=None).token()
        assert "audience" not in server.requests[-1].form()

    def test_caches_until_the_skew_window(self, server):
        clock = Clock()
        mint(server, "tok-1", expires_in=3600)
        creds = credentials(server, clock)
        assert creds.token() == "tok-1"

        clock.now += 3600 - ClientCredentials.SKEW_SECONDS - 1
        assert creds.token() == "tok-1"
        assert len(server.requests) == 1

    def test_re_mints_once_inside_the_skew_window(self, server):
        clock = Clock()
        mint(server, "tok-1", expires_in=3600)
        mint(server, "tok-2", expires_in=3600)
        creds = credentials(server, clock)
        assert creds.token() == "tok-1"

        # Refresh-ahead: the token is still live, but not for long enough.
        clock.now += 3600 - ClientCredentials.SKEW_SECONDS
        assert creds.token() == "tok-2"
        assert len(server.requests) == 2

    def test_expiry_is_absolute_not_relative(self, server):
        # A duration cannot survive being cached: a relative expiry would still look
        # fresh an hour later because it was never anchored to a wall clock.
        clock = Clock()
        mint(server, "tok-1", expires_in=100)
        mint(server, "tok-2", expires_in=100)
        creds = credentials(server, clock)
        creds.token()
        clock.now += 1_000
        assert creds.token() == "tok-2"

    def test_refresh_mints_unconditionally(self, server):
        mint(server, "tok-1")
        mint(server, "tok-2")
        creds = credentials(server)
        assert creds.token() == "tok-1"
        assert creds.refresh() == "tok-2"
        assert creds.token() == "tok-2"

    def test_a_failed_mint_raises_unauthorized_carrying_the_body(self, server):
        server.respond(status=401, body=json.dumps({"error": "invalid_client"}))
        with pytest.raises(Unauthorized) as excinfo:
            credentials(server).token()
        assert excinfo.value.status == 401
        assert "invalid_client" in str(excinfo.value.details)

    def test_an_unreachable_issuer_is_transient_not_a_credential_failure(self):
        # A refused connection says nothing about whether the secret is good, so
        # classifying it as Unauthorized would dead-letter a job that should retry.
        creds = ClientCredentials(
            # 127.0.0.1:1 is reserved and never listening.
            token_url="http://127.0.0.1:1/oauth/token",
            client_id="cid",
            client_secret="sec",
        )
        with pytest.raises(TransportError):
            creds.token()

    def test_concurrent_callers_mint_once(self, server):
        # The steward's cache is a bare module global, sound only because a
        # serverless function is single-threaded. Under a threaded server every
        # in-flight thread races to mint at expiry.
        mint(server, "tok-1", times=1)
        server.always(
            status=200,
            body=json.dumps({"access_token": "tok-RACE", "expires_in": 3600}),
            headers={"Content-Type": "application/json"},
        )
        creds = credentials(server)

        results = []
        barrier = threading.Barrier(8)

        def worker():
            barrier.wait()
            results.append(creds.token())

        threads = [threading.Thread(target=worker) for _ in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert set(results) == {"tok-1"}
        assert len(server.requests) == 1


class TestBearerTokenAdmission:
    @pytest.mark.parametrize("bad", ["tok\n", " tok", "tok\r\nX-Admin: 1", "two words"])
    def test_refuses_a_token_that_cannot_be_a_header_value(self, bad):
        # `BEARER=$(cat token.txt)` keeps the newline. urllib3 v2 rejects the header
        # too — on the request, several frames from the BearerToken that caused it.
        with pytest.raises(ValueError, match="whitespace"):
            BearerToken(bad)

    def test_its_repr_does_not_print_the_token(self):
        assert "s3cret" not in repr(BearerToken("s3cret"))


class TestClientCredentialAdmission:
    def base(self, **overrides):
        kwargs = {
            "token_url": "https://auth.test/oauth/token",
            "client_id": "cid",
            "client_secret": "sec",
        }
        kwargs.update(overrides)
        return kwargs

    @pytest.mark.parametrize("bad", ["sec\n", "sec ", "s ec"])
    def test_refuses_a_client_secret_with_whitespace(self, bad):
        # THE field failure: `TEMPER_M2M_CLIENT_SECRET=$(cat secret.txt)` keeps the
        # trailing newline, and the issuer answers `invalid_client` either way.
        with pytest.raises(ValueError, match="whitespace"):
            ClientCredentials(**self.base(client_secret=bad))

    def test_refuses_a_client_id_in_the_client_secret_slot(self):
        # A temper client_id is `tmpr_` + base64url; a temper secret is base64url with
        # no prefix, so a secret starting with those five characters is a swap.
        with pytest.raises(ValueError, match="swapped"):
            ClientCredentials(**self.base(client_secret="tmpr_AbCdEf0123456789"))

    def test_a_real_temper_client_id_is_still_fine_in_the_client_id_slot(self):
        creds = ClientCredentials(**self.base(client_id="tmpr_AbCdEf0123456789"))
        assert creds is not None

    @pytest.mark.parametrize("field", ["client_id", "audience"])
    def test_refuses_whitespace_in_the_other_form_fields(self, field):
        with pytest.raises(ValueError, match="whitespace"):
            ClientCredentials(**self.base(**{field: "value\n"}))

    def test_refuses_a_plaintext_token_url_off_the_loopback_interface(self):
        # The client_secret travels in this request body on every mint.
        with pytest.raises(ValueError, match="plaintext http"):
            ClientCredentials(**self.base(token_url="http://auth.test/oauth/token"))

    def test_the_plaintext_opt_out_is_explicit(self):
        creds = ClientCredentials(
            **self.base(token_url="http://auth.internal/oauth/token"),
            allow_insecure_http=True,
        )
        assert creds.can_refresh is True

    def test_refuses_a_token_url_carrying_userinfo(self):
        # It would otherwise ride in every TransportError that names the URL.
        with pytest.raises(ValueError, match="userinfo"):
            ClientCredentials(**self.base(token_url="https://id:s3cret@auth.test/oauth/token"))

    def test_its_repr_does_not_print_the_secret(self):
        text = repr(ClientCredentials(**self.base(client_secret="s3cret")))
        assert "s3cret" not in text
        # The parts that are safe are still there, because that is what a repr is for.
        assert "cid" in text


class TestMintResponseHandling:
    """A 200 from the token endpoint is not the same thing as a token."""

    def test_a_redirect_is_refused_rather_than_followed_with_the_secret_in_hand(self, server):
        # urllib3 follows a 3xx by default, and following THIS one re-POSTs the form —
        # client_secret and all — to whatever origin the Location names.
        server.respond(status=302, headers={"Location": "http://127.0.0.1:1/elsewhere"})
        with pytest.raises(Unauthorized, match="refusing to re-send"):
            credentials(server).token()
        assert len(server.requests) == 1

    def test_an_oversized_body_is_refused_rather_than_buffered(self, server):
        # The mint holds the lock; a client pointed at something that is not a token
        # endpoint should not stream it into memory while every other thread waits.
        server.respond(status=200, body=b"x" * (MAX_TOKEN_RESPONSE_BYTES + 1024))
        with pytest.raises(Unauthorized, match="does not look like a token endpoint"):
            credentials(server).token()

    def test_an_html_body_is_a_credential_error_not_a_json_decode_error(self, server):
        # A captive portal or an SSO login page answers 200 with HTML.
        server.respond(status=200, body="<html>sign in</html>")
        with pytest.raises(Unauthorized, match="non-JSON body"):
            credentials(server).token()

    def test_a_json_array_is_refused(self, server):
        server.respond_json([1, 2, 3])
        with pytest.raises(Unauthorized, match="not an object"):
            credentials(server).token()

    @pytest.mark.parametrize("body", [{}, {"expires_in": 3600}, {"access_token": ""}])
    def test_a_missing_access_token_is_reported_not_raised_as_a_key_error(self, server, body):
        # `body["access_token"]` made this a KeyError out of `token()` — a defect
        # report about this package, for something the issuer did.
        server.respond_json(body)
        with pytest.raises(Unauthorized, match="no access_token"):
            credentials(server).token()

    def test_an_access_token_that_cannot_be_a_header_value_is_refused(self, server):
        # Unauthorized, not the ValueError the constructors raise: a ValueError from
        # this package means the CALLER passed something wrong, and here they did not.
        server.respond_json({"access_token": "tok\r\nX-Admin: 1", "expires_in": 3600})
        with pytest.raises(Unauthorized, match="Authorization header"):
            credentials(server).token()

    @pytest.mark.parametrize(
        "body", [{"access_token": "t"}, {"access_token": "t", "expires_in": "soon"}]
    )
    def test_a_missing_or_unusable_expires_in_is_reported(self, server, body):
        server.respond_json(body)
        with pytest.raises(Unauthorized, match="expires_in"):
            credentials(server).token()

    def test_an_already_expired_lifetime_is_refused_rather_than_cached(self, server):
        server.respond_json({"access_token": "t", "expires_in": 0})
        with pytest.raises(Unauthorized, match="already expired"):
            credentials(server).token()

    def test_a_non_bearer_token_type_is_refused(self, server):
        # It is about to go into an `Authorization: Bearer ...` header.
        server.respond_json({"access_token": "t", "token_type": "DPoP", "expires_in": 3600})
        with pytest.raises(Unauthorized, match="not Bearer"):
            credentials(server).token()

    def test_a_lowercase_bearer_is_fine_and_an_absent_one_is_too(self, server):
        server.respond_json({"access_token": "t1", "token_type": "bearer", "expires_in": 3600})
        assert credentials(server).token() == "t1"
        server.respond_json({"access_token": "t2", "expires_in": 3600})
        assert credentials(server).token() == "t2"


class TestMintFailureLeavesNoDeadToken:
    def test_a_failed_refresh_drops_the_token_the_server_already_rejected(self, server):
        # `refresh()` is called BECAUSE the server rejected the cached token. Keeping
        # it on a failed re-mint turns one legible 401 into a loop of them.
        mint(server, "tok-1")
        creds = credentials(server)
        assert creds.token() == "tok-1"

        server.respond(status=503, body="issuer down")
        with pytest.raises(Unauthorized):
            creds.refresh()

        mint(server, "tok-2")
        assert creds.token() == "tok-2"


def test_a_token_endpoint_that_accepts_and_never_answers_is_a_transport_failure():
    # The read timeout, proved against a socket that completes the handshake and then
    # says nothing — the shape a hung issuer actually takes. Without it this call
    # blocks on `self._lock` forever, and so does every other thread needing a token.
    import socket

    import urllib3

    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    try:
        port = listener.getsockname()[1]
        creds = ClientCredentials(
            token_url=f"http://127.0.0.1:{port}/oauth/token",
            client_id="cid",
            client_secret="sec",
            timeout=urllib3.Timeout(connect=2.0, read=0.25),
        )
        started = time.monotonic()
        with pytest.raises(TransportError):
            creds.token()
        assert time.monotonic() - started < 5.0
    finally:
        listener.close()


def test_the_mint_does_not_wait_forever_while_holding_the_lock():
    # urllib3's default is the socket default, which is no timeout at all — so a token
    # endpoint that accepts the connection and never answers blocks not one caller but
    # every thread that needs a token.
    creds = ClientCredentials(
        token_url="https://auth.test/oauth/token",
        client_id="cid",
        client_secret="sec",
    )
    assert creds._timeout is ClientCredentials.DEFAULT_TIMEOUT
    assert ClientCredentials.DEFAULT_TIMEOUT.read_timeout is not None
