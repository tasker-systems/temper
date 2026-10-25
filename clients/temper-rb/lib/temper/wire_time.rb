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
  # `Time` (`openapi_types`), in every model nested in it, whether the caller passes the nested part
  # as its model or as a hash; in a query, every `Time` value, since a query is text on the URL and
  # never stored. A `Time` a caller places in free-form JSON (an artifact's `content`, a shape's
  # `schema`, any `Object` attribute) is written by the json gem exactly as the generated core
  # writes it, so the content a caller commits is stored, and hashed, as the caller wrote it. A
  # whole body passed as a bare hash, not as its request model, names no declared type and is
  # written by the generated core unchanged.
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
    # (`_to_hash` copies every hash and array it writes, so no caller object is changed.)
    def self.stamp_declared(value, written)
      if value.is_a?(Array) && written.is_a?(Array)
        # The core writes an array without its nils (`_to_hash`), so the two align without them.
        value.compact.zip(written) { |v, w| stamp_declared(v, w) }
      elsif written.is_a?(Hash) && value.class.respond_to?(:openapi_types)
        stamp_fields(value.class, written) { |attr| value.public_send(attr) }
      end
      written
    end

    # Each field of `model_class` that `written` holds: stamped where the class declares it a
    # timestamp, and entered by its declared type (`stamp_slot`). The block gives the caller's
    # value for an attribute, which for a model is the attribute and for a hash is its entry.
    def self.stamp_fields(model_class, written)
      model_class.openapi_types.each do |attr, type|
        key = written_key(model_class, attr, written)
        next if key.nil?

        written[key] = stamp(written[key]) if type == :Time
        stamp_slot(type, yield(attr, key), written[key])
      end
    end

    # A slot declared `type`, holding the caller's `value` and written as `written`. A model is
    # entered as itself. A hash is entered when the slot declares a model, so a part a caller writes
    # as a hash (`after: { created: …, artifact_id: … }`, the idiom the SDK's own surfaces use) is
    # stamped as its model would be. An array is entered element by element. A slot declared
    # free-form (`Object`) is never entered, whatever it holds, a generated model included.
    def self.stamp_slot(type, value, written)
      return if type.to_s == 'Object'

      if value.class.respond_to?(:openapi_types)
        stamp_declared(value, written)
      elsif value.is_a?(Array) && written.is_a?(Array)
        stamp_elements(type, value, written)
      elsif value.is_a?(Hash) && written.is_a?(Hash)
        stamp_hash(type, written)
      end
    end

    # The elements of an array slot, each entered by the element type `Array<…>` declares.
    def self.stamp_elements(type, value, written)
      element = type.to_s[/\AArray<(\w+)>\z/, 1]
      value.compact.zip(written) { |v, w| stamp_slot(element, v, w) } if element
    end

    # A hash in a slot that declares a model, entered as that model's fields; any other hash (a
    # free-form `Object`, a map) is left whole.
    def self.stamp_hash(type, written)
      model_class = declared_model(type)
      stamp_fields(model_class, written) { |_attr, key| written[key] } if model_class
    end

    # The generated model class `type` names, or nil when it names none (a scalar, `Object`, a map).
    def self.declared_model(type)
      name = type.to_s
      return nil unless name.match?(/\A[A-Z]\w*\z/) && Temper::Generated.const_defined?(name, false)

      model_class = Temper::Generated.const_get(name, false)
      model_class if model_class.respond_to?(:openapi_types)
    end

    # The key `written` holds `attr` under: the wire name the core writes for a model, or, for a
    # hash the caller wrote, the attribute or wire name as a symbol or a string.
    def self.written_key(model_class, attr, written)
      wire = model_class.attribute_map[attr]
      [wire, attr, wire.to_s, attr.to_s].find { |k| written.key?(k) }
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
