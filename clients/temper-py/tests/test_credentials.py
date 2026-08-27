import json
import threading
from typing import Any

import pytest

from temper import BearerToken, ClientCredentials, TransportError, Unauthorized


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
