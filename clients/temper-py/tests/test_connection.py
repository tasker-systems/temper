"""One connection per process, one token per call."""

from __future__ import annotations

import gzip
import json
import threading

import pytest

import temper
from temper import BearerToken, Client
from temper.connection import TokenScopedConfiguration
from tests.fixtures import profile_with_entitlements


def test_memoizes_one_api_client_per_process():
    temper.configure(base_url="https://api.test")
    first = temper.api_client()
    assert temper.api_client() is first


def test_reset_connection_drops_the_memo_so_a_forked_worker_builds_fresh_sockets():
    temper.configure(base_url="https://api.test")
    first = temper.api_client()
    temper.reset_connection()
    assert temper.api_client() is not first


def test_raises_when_unconfigured_rather_than_building_a_useless_client():
    with pytest.raises(RuntimeError, match="configure"):
        temper.api_client()


@pytest.mark.parametrize("bad", ["", "temperkb.io", "ftp://temperkb.io", None])
def test_rejects_a_base_url_that_is_not_an_absolute_http_origin(bad):
    with pytest.raises(ValueError):
        temper.configure(base_url=bad)


def test_strips_a_trailing_slash_from_the_origin():
    # The generated core joins `host` with a path that already starts with `/`, so a
    # trailing slash yields `//api/...` — a 404 that reads like a missing route rather
    # than like a config typo.
    temper.configure(base_url="https://api.test/")
    assert temper.api_client().configuration.host == "https://api.test"


def test_stamps_the_surface_header_once_on_the_client():
    temper.configure(base_url="https://api.test")
    assert temper.api_client().default_headers["X-Temper-Surface"] == "sdk"


def test_the_device_id_header_is_present_only_when_configured():
    temper.configure(base_url="https://api.test")
    assert "X-Temper-Device-Id" not in temper.api_client().default_headers

    temper.configure(base_url="https://api.test", device_id="dev-1")
    assert temper.api_client().default_headers["X-Temper-Device-Id"] == "dev-1"


def test_urllib3_is_not_left_to_retry_on_its_own():
    # urllib3's default Retry(3) allows PUT and DELETE, so a read timeout on a temper
    # write would be re-sent one layer BELOW the seam that exists to decide exactly
    # that. Retry policy belongs in Client.call, which knows about idempotency.
    temper.configure(base_url="https://api.test")
    assert temper.api_client().configuration.retries is False


class TestCallScopedToken:
    def test_the_token_is_not_on_the_configuration_object(self):
        temper.configure(base_url="https://api.test")
        assert temper.api_client().configuration.access_token is None

    def test_with_token_binds_and_unwinds(self):
        with temper.with_token("a"):
            assert temper.current_token() == "a"
            with temper.with_token("b"):
                assert temper.current_token() == "b"
            assert temper.current_token() == "a"
        assert temper.current_token() is None

    def test_a_direct_assignment_is_refused_rather_than_silently_global(self):
        # Setting it here would set it for whatever context happens to be current,
        # which is never what a caller means.
        config = TokenScopedConfiguration(host="https://api.test")
        with pytest.raises(AttributeError, match="with_token"):
            config.access_token = "leaked"

    def test_a_fresh_thread_starts_with_no_token(self):
        # A ContextVar is thread-local for free, which is the property the whole
        # process-global-connection design rests on.
        seen = {}

        def worker():
            seen["token"] = temper.current_token()

        with temper.with_token("outer"):
            thread = threading.Thread(target=worker)
            thread.start()
            thread.join()

        assert seen["token"] is None


def test_inflates_a_gzip_encoded_response_body(server):
    # The gem hit this as issue #446: its adapter bypassed Net::HTTP's transparent
    # decompression and every read returned nil. urllib3 decodes Content-Encoding
    # itself, but the client sets `preload_content=False` — the mode where a caller
    # CAN opt out of decoding — so it is worth pinning that we did not.
    body = json.dumps(profile_with_entitlements()).encode("utf-8")
    server.respond(
        status=200,
        body=gzip.compress(body),
        headers={"Content-Type": "application/json", "Content-Encoding": "gzip"},
    )
    temper.configure(base_url=server.url)

    profile = Client(BearerToken("tok")).whoami()
    assert profile.slug == "j-cole-taylor"
