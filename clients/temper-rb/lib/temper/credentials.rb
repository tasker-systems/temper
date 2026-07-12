# frozen_string_literal: true

require 'faraday'
require 'json'
require 'uri'

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

    # A client_credentials machine principal -- a Sidekiq worker.
    #
    # Works against BOTH issuers a temper instance can be fronted by:
    #
    #   * Auth0 (`temper admin machine provision`), where `token_url` is your Auth0
    #     tenant's /oauth/token and `audience` must equal the API's AUTH_AUDIENCE.
    #   * temper's own AS (`temper admin machine issue`, a `tmpr_*` client id), where
    #     `token_url` is your own instance's /oauth/token and `audience` is omitted --
    #     that AS mints with its server-side AS_AUDIENCE and ignores a request-supplied
    #     one entirely.
    #
    # Ported from packages/agent-workflows/steward/agent/lib/temper-auth.ts, the
    # machine-principal caller already running in production: the same TEMPER_M2M_*
    # inputs, `requireEnv` semantics (throw, never default), and a cache keyed on an
    # ABSOLUTE expires_at with a 60s skew.
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
      TOKEN_REQUEST_CONTENT_TYPE = 'application/x-www-form-urlencoded'

      # `audience` is optional because it is Auth0's, not the protocol's. Omit it for a
      # temper-issued credential.
      def initialize(token_url:, client_id:, client_secret:, audience: nil, clock: -> { Time.now })
        @token_url = require_value(token_url, 'token_url')
        @client_id = require_value(client_id, 'client_id')
        @client_secret = require_value(client_secret, 'client_secret')
        @audience = audience.nil? ? nil : require_value(audience, 'audience')
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
          req.headers['Content-Type'] = TOKEN_REQUEST_CONTENT_TYPE
          req.body = token_request_body
        end
        return response if response.success?

        raise Unauthorized.new("token mint failed (#{response.status})",
                               status: response.status, details: response.body)
      end

      # RFC 6749 §4 mandates form encoding at the token endpoint. Auth0 also accepts
      # JSON, which is why this sent JSON while Auth0 was the only issuer it faced --
      # and why every test stayed green. Temper's own AS reads the body with
      # `req.formData()`, so a JSON mint never reaches its client_credentials branch.
      # Form-encoding is what both issuers accept.
      def token_request_body
        params = {
          grant_type: 'client_credentials',
          client_id: @client_id,
          client_secret: @client_secret
        }
        # Auth0 requires it; temper's AS ignores a request-supplied audience and mints
        # with its own AS_AUDIENCE. Sending an empty one would be a lie, so omit it.
        params[:audience] = @audience unless @audience.nil?
        URI.encode_www_form(params)
      end
    end
  end
end
