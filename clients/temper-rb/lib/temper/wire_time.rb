# frozen_string_literal: true

require 'time'

module Temper
  # Every timestamp the contract declares in a request is written as RFC 3339 with microseconds,
  # the form it declares for `format: date-time` and the precision the server stores.
  #
  # The generated core writes a `Time` as it finds it. A body is written with `Hash#to_json`, which
  # writes a `Time` as `Time#to_s` ("2026-10-25 09:58:12 UTC"), and Faraday writes a query value
  # with `to_s` too. Both drop the sub-seconds, and neither is RFC 3339. So a position read from a
  # page (`AskResponse#_next`, whose `created` the client parses into a `Time`) and sent back as
  # `after` would not name the row it named, and a walk could not go on from it.
  #
  # Only declared timestamps are rewritten. In a body, that is an attribute its model declares a
  # `Time` (`openapi_types`), in the models nested in it too; in a query, every `Time` value, since
  # a query is text on the URL and never stored. A `Time` a caller places in free-form JSON (an
  # artifact's `content`, a shape's `schema`, any `Object` attribute) is left to the json gem as
  # before, so the content a caller commits is stored, and hashed, as it was.
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

    # `value` as its RFC 3339 text if it is a `Time` or `DateTime`; anything else as it is.
    def self.stamp(value)
      case value
      when Time, DateTime then value.iso8601(DIGITS)
      else value
      end
    end

    # `written`, the generated core's hash of `value`, with every declared timestamp in it stamped
    # in place. It enters a model, and an array, and nothing else: a free-form value is left whole.
    def self.stamp_declared(value, written)
      if value.is_a?(Array) && written.is_a?(Array)
        # The core writes an array without its nils (`_to_hash`), so the two align without them.
        value.compact.zip(written) { |v, w| stamp_declared(v, w) }
      elsif written.is_a?(Hash) && value.class.respond_to?(:openapi_types)
        stamp_attributes(value, written)
      end
      written
    end

    # Each attribute of `model` that `written` holds: stamped where the model declares it a
    # timestamp, and entered where it holds a model or an array.
    def self.stamp_attributes(model, written)
      model.class.openapi_types.each do |attr, type|
        param = model.class.attribute_map[attr]
        next unless written.key?(param)

        written[param] = stamp(written[param]) if type == :Time
        stamp_declared(model.public_send(attr), written[param])
      end
    end

    # The query parameters, every `Time` or `DateTime` among their values stamped.
    def self.query(params)
      params.transform_values { |v| v.is_a?(Array) ? v.map { |e| stamp(e) } : stamp(v) }
    end

    # Prepended to the generated `ApiClient`.
    module ApiClientPatch
      def object_to_hash(obj)
        WireTime.stamp_declared(obj, super)
      end

      def build_request(http_method, path, request, opts = {})
        query_params = opts[:query_params]
        return super if query_params.nil?

        super(http_method, path, request, opts.merge(query_params: WireTime.query(query_params)))
      end
    end

    Temper::Generated::ApiClient.prepend(ApiClientPatch)
  end
end
