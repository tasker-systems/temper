import json
import threading

import pytest

import temper
from temper import BearerToken, Client, NotFound, ServerError, TransportError, Unauthorized
from temper.credentials import ClientCredentials
from temper.generated.api.profile_api import ProfileApi
from temper.generated.exceptions import ApiException
from tests.fixtures import profile_with_entitlements, search_response


@pytest.fixture
def configured(server):
    temper.configure(base_url=server.url)
    return server


# `GET /api/profile` is the seam's stand-in operation: it is declared
# `security: [bearer_auth]`, so it is one of the operations whose Authorization header
# the generated core actually emits. `GET /api/health` is unauthenticated — a seam test
# written against it would assert the retry policy while silently proving nothing about
# the token.
def profile(api):
    return ProfileApi(api).get_profile()


PROFILE = profile_with_entitlements()


def no_sleep(_attempt: int) -> None:
    """The backoff seam exists so the retry policy is testable without wall-clock."""


class TestCallSeam:
    def test_returns_the_deserialized_body(self, configured):
        configured.respond_json(PROFILE)
        result = Client(BearerToken("tok")).call(profile, idempotent=True)
        assert result.slug == "j-cole-taylor"

    def test_authenticates_with_the_credential_and_marks_the_surface(self, configured):
        configured.respond_json(PROFILE)
        Client(BearerToken("tok")).call(profile, idempotent=True)

        sent = configured.requests[-1]
        assert sent.headers["authorization"] == "Bearer tok"
        # It names the KIND of surface, never the language — the gem and temper-ts
        # both send this same value.
        assert sent.headers["x-temper-surface"] == "sdk"

    def test_a_read_retries_a_transient_failure(self, configured):
        configured.respond(status=503, body="boom", times=2)
        configured.respond_json(PROFILE)
        result = Client(BearerToken("tok"), backoff=no_sleep).call(profile, idempotent=True)
        assert result.slug == "j-cole-taylor"
        assert len(configured.requests) == 3

    def test_a_read_gives_up_after_max_attempts(self, configured):
        configured.always(status=503, body="boom")
        with pytest.raises(ServerError):
            Client(BearerToken("tok"), backoff=no_sleep).call(profile, idempotent=True)
        assert len(configured.requests) == temper.MAX_READ_ATTEMPTS

    def test_a_write_is_never_auto_retried(self, configured):
        # A 503 does not mean the write did not land. Re-sending it is how a ledger
        # gets a duplicate act.
        configured.always(status=503, body="boom")
        with pytest.raises(ServerError):
            Client(BearerToken("tok"), backoff=no_sleep).call(profile)
        assert len(configured.requests) == 1

    def test_a_permanent_failure_is_never_retried_even_for_a_read(self, configured):
        configured.always(status=404, body='{"error":{"message":"gone"}}')
        with pytest.raises(NotFound):
            Client(BearerToken("tok"), backoff=no_sleep).call(profile, idempotent=True)
        assert len(configured.requests) == 1

    def test_a_transport_failure_is_transient(self, server):
        # Nothing is listening on 127.0.0.1:1, so this never reaches an HTTP status.
        temper.configure(base_url="http://127.0.0.1:1")
        with pytest.raises(TransportError):
            Client(BearerToken("tok"), backoff=no_sleep).call(profile)

    def test_the_backoff_grows_with_the_attempt(self, configured):
        configured.always(status=503, body="boom")
        attempts: list[int] = []
        with pytest.raises(ServerError):
            Client(BearerToken("tok"), backoff=attempts.append).call(profile, idempotent=True)
        assert attempts == [1, 2]


class TestCredentialRepair:
    def _minting_credentials(self, server):
        # The issuer and the API share this recording server; the mint is the only POST
        # to /oauth/token, so the assertions below read it by path.
        return ClientCredentials(
            token_url=f"{server.url}/oauth/token",
            client_id="cid",
            client_secret="sec",
        )

    def test_one_401_buys_exactly_one_re_mint(self, configured):
        creds = self._minting_credentials(configured)
        configured.respond_json({"access_token": "tok-1", "expires_in": 3600})  # first mint
        configured.respond(status=401, body='{"error":{"message":"expired"}}')
        configured.respond_json({"access_token": "tok-2", "expires_in": 3600})  # re-mint
        configured.respond_json(PROFILE)

        result = Client(creds, backoff=no_sleep).call(profile)
        assert result.slug == "j-cole-taylor"

        api_calls = [r for r in configured.requests if r.path != "/oauth/token"]
        assert [r.headers["authorization"] for r in api_calls] == ["Bearer tok-1", "Bearer tok-2"]

    def test_a_401_is_repaired_for_a_write_too(self, configured):
        # Re-authenticating is not re-submitting: the first attempt was rejected
        # before it could have landed, so this is not the retry a write forbids.
        creds = self._minting_credentials(configured)
        configured.respond_json({"access_token": "tok-1", "expires_in": 3600})
        configured.respond(status=401, body='{"error":{"message":"expired"}}')
        configured.respond_json({"access_token": "tok-2", "expires_in": 3600})
        configured.respond_json(PROFILE)

        assert Client(creds, backoff=no_sleep).call(profile).slug == "j-cole-taylor"

    def test_a_401_that_survives_a_fresh_token_is_a_real_authorization_failure(self, configured):
        creds = self._minting_credentials(configured)
        configured.respond_json({"access_token": "tok-1", "expires_in": 3600})
        configured.respond(status=401, body='{"error":{"message":"expired"}}')
        configured.respond_json({"access_token": "tok-2", "expires_in": 3600})
        configured.always(status=401, body='{"error":{"message":"revoked"}}')

        with pytest.raises(Unauthorized) as excinfo:
            Client(creds, backoff=no_sleep).call(profile, idempotent=True)
        # Retrying it further would only bury the error.
        assert "revoked" in str(excinfo.value)

    def test_a_bearer_token_gets_the_servers_401_back_untouched(self, configured):
        # BearerToken cannot mint. Raising the client's own "cannot refresh" here would
        # replace temper's real answer — the body a human is trying to read.
        configured.always(status=401, body='{"error":{"message":"the servers own words"}}')
        with pytest.raises(Unauthorized) as excinfo:
            Client(BearerToken("tok"), backoff=no_sleep).call(profile, idempotent=True)
        assert str(excinfo.value) == "the servers own words"
        assert len(configured.requests) == 1


class TestConveniences:
    def test_whoami_reads_the_profile(self, configured):
        configured.respond_json(profile_with_entitlements())
        profile = Client(BearerToken("tok")).whoami()
        assert profile.slug == "j-cole-taylor"
        assert profile.entitlements.system_access is True
        assert configured.requests[-1].path == "/api/profile"

    def test_search_names_the_field_query_not_q(self, configured):
        configured.respond_json(search_response())
        Client(BearerToken("tok")).search("incidents")
        assert configured.requests[-1].json()["query"] == "incidents"


class TestConcurrency:
    def test_one_connection_serves_concurrent_callers_with_their_own_identity(self, configured):
        # The whole point of a call-scoped token over a process-global connection: two
        # threads share one pool and never see each other's credential.
        configured.always(
            status=200,
            body=json.dumps(PROFILE),
            headers={"Content-Type": "application/json"},
        )
        seen = {}
        barrier = threading.Barrier(2)

        def worker(name):
            client = Client(BearerToken(f"tok-{name}"))
            barrier.wait()
            client.call(profile, idempotent=True)
            seen[name] = temper.current_token()

        threads = [threading.Thread(target=worker, args=(n,)) for n in ("a", "b")]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        sent = sorted(r.headers["authorization"] for r in configured.requests)
        assert sent == ["Bearer tok-a", "Bearer tok-b"]
        # The scope is unwound on the way out, in every thread.
        assert seen == {"a": None, "b": None}


def test_the_underlying_failure_stays_on_the_traceback(configured):
    # `call` maps the generated core's exception into the tree and raises the mapped
    # one, which loses the original unless it is chained explicitly — and the original
    # is the frame that says what actually happened on the wire.
    configured.always(status=503, body="boom")
    with pytest.raises(ServerError) as excinfo:
        Client(BearerToken("tok"), backoff=no_sleep).call(profile)
    assert isinstance(excinfo.value.__cause__, ApiException)
