# frozen_string_literal: true

require 'faraday'
require 'faraday/gzip'
require 'faraday/net_http_persistent'
require 'uri'

module Temper
  # Thread.current[] is FIBER-local in Ruby (Thread#thread_variable_get is the
  # thread-local one). Fiber[] would read better but arrived in 3.2, and the
  # gem's floor is 3.1.
  TOKEN_KEY = :temper_access_token
  private_constant :TOKEN_KEY

  class << self
    # One ApiClient per process => one Faraday connection => one
    # net-http-persistent per-thread pool.
    #
    # A fresh Configuration + ApiClient per request is the obvious fix for the
    # generated singletons' single shared access_token, and it is a trap: fresh
    # client, fresh connection, TLS handshake per request. Instead the connection
    # is process-global and the TOKEN is call-scoped, via access_token_getter.
    def api_client
      connection_mutex.synchronize { @api_client ||= build_api_client }
    end

    # Call from Puma's on_worker_boot and from Sidekiq.configure_server, so a
    # forked worker never inherits its parent's sockets.
    def reset_connection!
      connection_mutex.synchronize { @api_client = nil }
      nil
    end

    def current_token
      Thread.current[TOKEN_KEY]
    end

    def with_token(token)
      previous = Thread.current[TOKEN_KEY]
      Thread.current[TOKEN_KEY] = token
      yield
    ensure
      Thread.current[TOKEN_KEY] = previous
    end

    private

    def connection_mutex
      @connection_mutex ||= Mutex.new
    end

    def build_api_client
      generated = build_generated_config
      Generated::ApiClient.new(generated).tap do |client|
        client.default_headers['X-Temper-Surface'] = 'sdk'
        client.default_headers['X-Temper-Device-Id'] = config.device_id if config.device_id
      end
    end

    def build_generated_config
      Generated::Configuration.new.tap do |c|
        apply_endpoint(c, base_uri)
        c.access_token_getter = -> { Temper.current_token }
        configure_faraday(c)
      end
    end

    def base_uri
      raise ArgumentError, 'Temper.config.base_url is not set' if config.base_url.nil?

      URI.parse(config.base_url)
    end

    def apply_endpoint(generated_config, base)
      generated_config.scheme = base.scheme
      generated_config.host = host_with_port(base)
      generated_config.base_path = base.path
    end

    # `configure_faraday_connection` REGISTERS the block; `configure_connection` is
    # the invoker the generated ApiClient calls with the connection.
    #
    # It runs AFTER build_connection sets conn.adapter(Faraday.default_adapter), so
    # setting the adapter here replaces it. An adapter set via `configure_middleware`
    # runs BEFORE, and would be silently overwritten by the default -- costing a TLS
    # handshake per request while nothing fails.
    #
    # `conn.request :gzip` installs faraday-gzip's decompression middleware. It sets
    # Accept-Encoding: gzip,deflate and inflates the response in Faraday's
    # on_complete -- OUTSIDE Net::HTTP. That is the whole point: the
    # net_http_persistent adapter bypasses Net::HTTP's transparent gzip handling, so
    # without this a `Content-Encoding: gzip` body reaches the deserializer as raw
    # compressed bytes and every read returns nil (issue #446). Order is irrelevant
    # -- Faraday keeps the adapter last regardless of call order -- but registering
    # the request middleware before the adapter reads in stack order.
    def configure_faraday(generated_config)
      generated_config.configure_faraday_connection do |conn|
        conn.request :gzip
        conn.adapter :net_http_persistent, pool_size: 5
      end
    end

    def host_with_port(base)
      return base.host if base.port.nil? || base.port == base.default_port

      "#{base.host}:#{base.port}"
    end
  end
end
