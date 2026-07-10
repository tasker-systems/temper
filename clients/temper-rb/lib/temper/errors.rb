# frozen_string_literal: true

require 'json'

module Temper
  class Error < StandardError
    attr_reader :status, :code, :details

    def initialize(message = nil, status: nil, code: nil, details: nil)
      super(message)
      @status = status
      @code = code
      @details = details
    end
  end

  # Let these escape: Sidekiq retries a job whose exception escapes.
  class TransientError < Error; end

  class RateLimited < TransientError
    attr_reader :retry_after

    def initialize(message = nil, retry_after: nil, **kwargs)
      super(message, **kwargs)
      @retry_after = retry_after
    end
  end

  class ServerError < TransientError; end
  class ConnectionError < TransientError; end

  # Rescue these: retrying will not help. Dead-letter them.
  class PermanentError < Error; end

  class Unauthorized < PermanentError; end
  class Forbidden < PermanentError; end
  class SystemAccessRequired < Forbidden; end
  class NotFound < PermanentError; end
  class Conflict < PermanentError; end
  class BadRequest < PermanentError; end

  # Translates the generated core's one flat ApiError into the tree above.
  #
  # The split is load-bearing rather than decorative: a 409 classified transient
  # spins a Sidekiq job forever, and a 503 classified permanent is silently
  # dropped.
  module ErrorMapper
    SYSTEM_ACCESS_REQUIRED = 'SYSTEM_ACCESS_REQUIRED'

    # 422 is declared on no operation but is what a serde rejection surfaces as.
    # 403 and 429 are not here: each needs more than the status to classify.
    STATUS_CLASSES = {
      400 => BadRequest,
      401 => Unauthorized,
      404 => NotFound,
      409 => Conflict,
      422 => BadRequest
    }.freeze

    module_function

    def call(api_error)
      status = api_error.code
      code, message, details = parse_envelope(api_error.response_body)
      # ApiError#message decorates itself with status/headers/body when it has
      # them. A transport failure has none, so this fallback stays clean.
      message ||= api_error.message
      kwargs = { status: status, code: code, details: details }

      # A nil status means the generated ApiClient rescued a Faraday transport
      # failure (timeout / connection refused) and re-raised it code-less.
      return ConnectionError.new(message, **kwargs) if status.nil?

      build(status, message, code, kwargs, api_error)
    end

    def build(status, message, code, kwargs, api_error)
      # 403 discriminates on error.code; 429 carries Retry-After. Everything else
      # classifies off the status alone.
      return forbidden(code, message, kwargs) if status == 403
      return RateLimited.new(message, retry_after: retry_after_of(api_error), **kwargs) if status == 429

      klass = STATUS_CLASSES[status] || (server_error?(status) ? ServerError : Error)
      klass.new(message, **kwargs)
    end

    def server_error?(status)
      (500..599).cover?(status)
    end

    def forbidden(code, message, kwargs)
      return SystemAccessRequired.new(message, **kwargs) if code == SYSTEM_ACCESS_REQUIRED

      Forbidden.new(message, **kwargs)
    end

    # The server speaks exactly one envelope: {"error":{code,message,details}}.
    # Anything else -- an HTML 502 from a proxy, an undeclared 500 -- degrades to
    # a raw body on #details rather than raising inside the error path.
    #
    # 422/429/500/503 are declared on NO operation, so those bodies are parsed
    # opportunistically and classified off the raw HTTP status.
    def parse_envelope(body)
      return [nil, nil, nil] if body.nil? || body.to_s.empty?

      parsed = JSON.parse(body.to_s)
      error = parsed.is_a?(Hash) ? parsed['error'] : nil
      return [nil, nil, body] unless error.is_a?(Hash)

      [error['code'], error['message'], error['details']]
    rescue JSON::ParserError
      [nil, nil, body]
    end

    def retry_after_of(api_error)
      headers = api_error.response_headers || {}
      raw = headers['Retry-After'] || headers['retry-after']
      raw && Integer(raw, exception: false)
    end
  end
end
