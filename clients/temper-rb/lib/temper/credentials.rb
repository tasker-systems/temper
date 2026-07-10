# frozen_string_literal: true

require 'faraday'
require 'json'

module Temper
  # Two strategies behind one interface. Precedence is not discovered from the
  # environment -- it is the caller's explicit choice. That avoids, structurally,
  # the drift the steward's temper-auth.ts header documents: its schedules went
  # Connect-first while its MCP connection went M2M-first, so on the Auth0-fronted
  # prod instance the schedules' REST calls silently failed while MCP worked.
  module Credentials
    # A token the caller already holds -- a Puma request serving a signed-in user.
    # No I/O, no refresh.
    class BearerToken
      def initialize(token)
        raise ArgumentError, 'token must be a non-empty String' unless token.is_a?(String) && !token.empty?

        @token = token
      end

      attr_reader :token

      def refresh!
        raise Unauthorized.new('BearerToken cannot refresh; mint a new token upstream', status: 401)
      end
    end

    # An Auth0 client_credentials machine principal -- a Sidekiq worker.
    #
    # Ported from packages/agent-workflows/steward/agent/lib/temper-auth.ts, the
    # machine-principal caller already running in production: the same four
    # TEMPER_M2M_* inputs, `requireEnv` semantics (throw, never default), and a
    # cache keyed on an ABSOLUTE expires_at with a 60s skew.
    #
    # Two deliberate divergences from that reference:
    #
    #   * The cache is mutex-guarded. The steward's is a bare module global, sound
    #     only because a serverless function is single-threaded. Under Puma every
    #     in-flight thread races to mint at expiry.
    #   * #refresh! exists. Refresh-ahead-of-expiry alone is insufficient: the
    #     steward resolves a token once per tick, so a tick outliving its cached
    #     token takes a 401 nothing recovers. A Sidekiq job holding a token across
    #     a long unit of work has precisely that bug. Re-mint ON 401.
    class ClientCredentials
      SKEW_SECONDS = 60

      def initialize(token_url:, client_id:, client_secret:, audience:, clock: -> { Time.now })
        @token_url = require_value(token_url, 'token_url')
        @client_id = require_value(client_id, 'client_id')
        @client_secret = require_value(client_secret, 'client_secret')
        @audience = require_value(audience, 'audience')
        @clock = clock
        @mutex = Mutex.new
        @token = nil
        @expires_at = nil
      end

      def token
        @mutex.synchronize do
          mint! if expired?
          @token
        end
      end

      def refresh!
        @mutex.synchronize { mint! }
      end

      private

      def require_value(value, name)
        raise ArgumentError, "#{name} must be a non-empty String" unless value.is_a?(String) && !value.empty?

        value
      end

      def expired?
        @token.nil? || @clock.call.to_f >= (@expires_at - SKEW_SECONDS)
      end

      # Caller holds @mutex.
      def mint!
        body = JSON.parse(post_token_request.body)
        @token = body.fetch('access_token')
        # Absolute, not relative: a duration cannot survive being cached.
        @expires_at = @clock.call.to_f + body.fetch('expires_in').to_i
        @token
      end

      def post_token_request
        response = Faraday.post(@token_url) do |req|
          req.headers['Content-Type'] = 'application/json'
          req.body = token_request_body
        end
        return response if response.success?

        raise Unauthorized.new("token mint failed (#{response.status})",
                               status: response.status, details: response.body)
      end

      # `audience` must equal the API's configured AUTH_AUDIENCE, or the minted
      # token fails validation before normalize_machine ever runs.
      def token_request_body
        JSON.generate(
          grant_type: 'client_credentials',
          client_id: @client_id,
          client_secret: @client_secret,
          audience: @audience
        )
      end
    end
  end
end
