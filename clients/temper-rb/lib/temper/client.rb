# frozen_string_literal: true

module Temper
  # A cheap façade over the process-global connection. Holds a credential and
  # nothing else; constructing one does no I/O, so a Puma request can build one
  # per user and a Sidekiq process can memoize one.
  class Client
    MAX_READ_ATTEMPTS = 3
    # 200ms, 400ms -- mirroring MAX_ATTEMPTS and the backoff in
    # crates/temper-client/src/http.rs.
    DEFAULT_BACKOFF = ->(attempt) { sleep(0.2 * (2**(attempt - 1))) }

    def initialize(credentials:, backoff: DEFAULT_BACKOFF)
      @credentials = credentials
      @backoff = backoff
    end

    # Assert the machine profile resolved, and report what it can reach, rather
    # than discovering it on the first write.
    #
    # Authentication is not authorization: a minted M2M token yields a
    # JIT-provisioned agent profile and nothing else, so without a cogmap write
    # grant and team membership every call authenticates cleanly and then 403s.
    def whoami
      call(idempotent: true) { |api| Generated::ProfileApi.new(api).get_profile }
    end

    # The one seam every surface module goes through.
    #
    #   idempotent: true  => a safe method. 5xx and transport failures retry.
    #   idempotent: false => a write. NEVER auto-retried.
    #
    # A 401 is repaired once, for reads and writes alike: re-authenticating is not
    # re-submitting. BearerToken#refresh! raises, so a bearer caller makes exactly
    # one request and the Unauthorized propagates out of refresh! itself.
    def call(idempotent: false, &block)
      attempt = 0
      reminted = false

      begin
        attempt += 1
        Temper.with_token(@credentials.token) { block.call(Temper.api_client) }
      rescue Generated::ApiError => e
        error = ErrorMapper.call(e)

        if repair_credentials?(error, reminted)
          reminted = true
          retry
        end

        raise error unless retryable_read?(error, idempotent, attempt)

        @backoff.call(attempt)
        retry
      end
    end

    private

    # BearerToken#refresh! raises Unauthorized, so a bearer caller never reaches
    # the retry -- the raise propagates straight out of refresh!.
    def repair_credentials?(error, reminted)
      return false unless error.is_a?(Unauthorized) && !reminted

      @credentials.refresh!
      true
    end

    def retryable_read?(error, idempotent, attempt)
      idempotent && error.is_a?(TransientError) && attempt < MAX_READ_ATTEMPTS
    end
  end
end
