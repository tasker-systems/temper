# frozen_string_literal: true

require 'time'

module Temper
  # Every timestamp the SDK writes into a request is RFC 3339 with microseconds, the form the
  # contract declares for `format: date-time` and the precision the server stores.
  #
  # The generated core writes a `Time` as it finds it. A body is written with `Hash#to_json`, which
  # writes a `Time` as `Time#to_s` ("2026-10-25 09:58:12 UTC"), and Faraday writes a query value
  # with `to_s` too. Both drop the sub-seconds, and neither is RFC 3339. So a position read from a
  # page (`AskResponse#_next`, whose `created` the client parses into a `Time`) and sent back as
  # `after` would not name the row it named, and a walk could not go on from it.
  #
  # It lives here, hand-written, because a regeneration would erase it from the generated core:
  # the Ruby generator has no option for how a `Time` is written, and a template override would
  # fork the whole client template. It wraps the two places the generated `ApiClient` turns a
  # request into text: `object_to_hash`, which every body passes through on its way to `to_json`,
  # and `build_request`, which hands the query parameters to Faraday.
  module WireTime
    # Microseconds: the precision a timestamp is stored at, so a position the server wrote is
    # written back as the same instant.
    DIGITS = 6

    # `value` with every `Time` or `DateTime` in it, at any depth of hashes and arrays, as its
    # RFC 3339 text. Everything else is left as it is.
    def self.written(value)
      case value
      when Time, DateTime then value.iso8601(DIGITS)
      when Hash then value.transform_values { |v| written(v) }
      when Array then value.map { |v| written(v) }
      else value
      end
    end

    # Prepended to the generated `ApiClient`.
    module ApiClientPatch
      def object_to_hash(obj)
        WireTime.written(super)
      end

      def build_request(http_method, path, request, opts = {})
        query_params = opts[:query_params]
        return super if query_params.nil?

        super(http_method, path, request, opts.merge(query_params: WireTime.written(query_params)))
      end
    end

    Temper::Generated::ApiClient.prepend(ApiClientPatch)
  end
end
