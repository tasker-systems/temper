"""One connection per process, one token per call."""

from __future__ import annotations

import gzip
import http.client as httplib
import inspect
import json
import threading

import pytest

import temper
from temper import BearerToken, Client
from temper.connection import (
    FORWARDABLE_SETTINGS,
    REFUSED_SETTINGS,
    TokenScopedConfiguration,
)
from temper.generated.configuration import Configuration as GeneratedConfiguration
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


@pytest.fixture
def _httplib_debuglevel():
    """Restore the CLASS attribute a `debug=True` would have set process-wide."""
    before = httplib.HTTPConnection.debuglevel
    yield
    httplib.HTTPConnection.debuglevel = before


class TestTheHazardSettingsPassthroughGuardsAgainst:
    """Characterizing the generated constructor, so the guard has something to point at.

    These assert on `Configuration` directly — NOT through `configure()` — because the
    point is what the underlying constructor does when it is reached, which is what
    makes the filter in `configure()` load-bearing rather than decorative.
    """

    def test_debug_true_turns_on_header_printing_for_the_whole_process(self, _httplib_debuglevel):
        assert httplib.HTTPConnection.debuglevel == 0
        GeneratedConfiguration(host="https://api.test", debug=True)
        # A CLASS attribute: every HTTP request in the process now prints its request
        # headers — `Authorization: Bearer ...` among them — to stdout, including
        # requests this package never made.
        assert httplib.HTTPConnection.debuglevel == 1

    def test_verify_ssl_false_reaches_cert_none(self):
        import ssl

        from temper.generated.rest import RESTClientObject

        rest = RESTClientObject(GeneratedConfiguration(host="https://api.test", verify_ssl=False))
        assert rest.pool_manager.connection_pool_kw["cert_reqs"] == ssl.CERT_NONE


class TestSettingsPassthrough:
    def test_debug_is_refused_and_the_process_wide_switch_stays_off(self, _httplib_debuglevel):
        with pytest.raises(ValueError, match="debuglevel"):
            temper.configure(base_url="https://api.test", debug=True)
        assert httplib.HTTPConnection.debuglevel == 0

    def test_verify_ssl_is_refused_and_names_the_setting_that_replaces_it(self):
        with pytest.raises(ValueError, match="ssl_ca_cert"):
            temper.configure(base_url="https://api.test", verify_ssl=False)

    def test_assert_hostname_is_refused(self):
        with pytest.raises(ValueError, match="tls_server_name"):
            temper.configure(base_url="https://api.test", assert_hostname=False)

    @pytest.mark.parametrize(
        "kwargs",
        [
            {"access_token": "leaked"},
            {"api_key": {"bearer_auth": "leaked"}},
            {"username": "u", "password": "p"},
        ],
    )
    def test_credentials_are_refused_because_they_are_call_scoped(self, kwargs):
        with pytest.raises(ValueError, match="Client"):
            temper.configure(base_url="https://api.test", **kwargs)

    @pytest.mark.parametrize("kwargs", [{"host": "https://elsewhere.test"}, {"retries": 5}])
    def test_settings_this_module_owns_are_refused_with_a_reason(self, kwargs):
        # Unfiltered these were a TypeError about duplicate keyword arguments, raised
        # from inside `_build()` on the first API call.
        with pytest.raises(ValueError, match="not configurable here"):
            temper.configure(base_url="https://api.test", **kwargs)

    def test_an_unknown_setting_fails_closed_and_names_what_is_allowed(self):
        # Fail-closed is the point: the generator adds constructor arguments on its
        # own schedule, and "unreviewed" is exactly how `debug` would have got in.
        with pytest.raises(TypeError, match="ssl_ca_cert"):
            temper.configure(base_url="https://api.test", not_a_real_setting=1)

    def test_the_settings_a_real_deployment_needs_do_reach_the_configuration(self):
        temper.configure(
            base_url="https://api.test",
            ssl_ca_cert="/etc/ssl/private-ca.pem",
            tls_server_name="api.internal",
            connection_pool_maxsize=7,
            proxy="http://proxy.internal:3128",
        )
        config = temper.api_client().configuration
        assert config.ssl_ca_cert == "/etc/ssl/private-ca.pem"
        assert config.tls_server_name == "api.internal"
        assert config.connection_pool_maxsize == 7
        assert config.proxy == "http://proxy.internal:3128"
        # And verification is still on, because nothing can turn it off from here.
        assert config.verify_ssl is True

    def test_a_refused_setting_is_caught_before_the_connection_is_installed(self):
        # `_build()` is lazy. Validated there, this would raise on the first API call
        # — and would have replaced a working connection with a broken one first.
        temper.configure(base_url="https://api.test")
        good = temper.current_connection()
        with pytest.raises(ValueError):
            temper.configure(base_url="https://api.test", debug=True)
        assert temper.current_connection() is good


class TestBaseUrlAdmission:
    def test_refuses_plaintext_http_to_a_non_loopback_host(self):
        # Every request would put the bearer token on the wire in the clear.
        with pytest.raises(ValueError, match="plaintext http"):
            temper.configure(base_url="http://temperkb.io")

    def test_loopback_is_exempt_because_that_is_what_a_dev_server_is(self):
        temper.configure(base_url="http://127.0.0.1:8080")
        assert temper.current_connection().base_url == "http://127.0.0.1:8080"

    def test_plaintext_elsewhere_takes_a_keyword_the_caller_has_to_write(self):
        temper.configure(base_url="http://temper.internal", allow_insecure_http=True)
        assert temper.current_connection().base_url == "http://temper.internal"

    def test_refuses_a_base_url_carrying_userinfo(self):
        with pytest.raises(ValueError, match="userinfo"):
            temper.configure(base_url="https://user:pass@temperkb.io")


class TestDeviceIdAdmission:
    def test_refuses_a_device_id_that_could_split_the_header_it_becomes(self):
        with pytest.raises(ValueError, match="whitespace"):
            temper.configure(base_url="https://api.test", device_id="dev-1\r\nX-Admin: 1")

    def test_refuses_a_device_id_with_a_trailing_newline(self):
        with pytest.raises(ValueError, match="whitespace"):
            temper.configure(base_url="https://api.test", device_id="dev-1\n")


def test_with_token_refuses_a_token_that_cannot_be_a_header_value():
    # The last seam before `Authorization: Bearer <this>`.
    with pytest.raises(ValueError, match="whitespace"), temper.with_token("tok\n"):
        pass


class TestTheFilterStaysPinnedToTheGeneratedConstructor:
    """The gate that makes `fails closed` more than an intention.

    `temper/generated/` is regenerated from `openapi.json` whenever the router
    changes, and the generator's `Configuration` gains and renames constructor
    arguments on its own schedule. A filter written once and never re-checked decays
    two ways: a REFUSED name that no longer exists stops refusing anything, and a NEW
    name arrives unreviewed — which is exactly how `debug` reached callers in the
    first place.
    """

    @staticmethod
    def _constructor_settings():
        parameters = inspect.signature(GeneratedConfiguration.__init__).parameters
        return {name for name in parameters if name != "self"}

    def test_every_name_in_the_filter_is_a_real_setting(self):
        # A rename upstream leaves a refusal that refuses nothing.
        assert (FORWARDABLE_SETTINGS | set(REFUSED_SETTINGS)) <= self._constructor_settings()

    def test_every_setting_is_either_forwarded_or_refused_with_a_reason(self):
        # THIS is the one that fails on the next regeneration that adds an argument,
        # and it is meant to: the reviewer decides which side it belongs on, once.
        unclassified = self._constructor_settings() - FORWARDABLE_SETTINGS - set(REFUSED_SETTINGS)
        assert unclassified == set(), (
            f"the generated Configuration grew {sorted(unclassified)}; add each to "
            f"FORWARDABLE_SETTINGS or to REFUSED_SETTINGS with the reason it is refused"
        )

    def test_nothing_is_on_both_lists(self):
        assert FORWARDABLE_SETTINGS.isdisjoint(REFUSED_SETTINGS)

    def test_every_refusal_explains_itself(self):
        # The message is the whole value of refusing rather than ignoring.
        assert all(reason.strip() for reason in REFUSED_SETTINGS.values())
