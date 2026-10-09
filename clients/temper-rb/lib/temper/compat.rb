# frozen_string_literal: true

module Temper
  # Call shapes the generated core no longer accepts but that released gems did. Each lives here,
  # hand-written, because regenerating the core would erase it from there.
  module Compat
    # `FacetsApi#retract_edge_facet` took the seven ActInput fields as required positionals,
    # because the contract once declared them path parameters. There was no placeholder for them
    # in the path, so the call dropped every one. The contract now declares them as query
    # parameters, and the generated methods take them in `opts`.
    #
    # The old nine-argument form (`edge_handle, property_id, invocation_id, correlation_id,
    # reasoning, confidence, rationale, persona, model[, opts]`) is still accepted: non-nil values
    # move into `opts`, where they are now actually sent. A caller that passed authorship without
    # `confidence` gets the server's 400, which the dropped fields used to hide.
    module RetractEdgeFacetPositionalActFields
      ACT_FIELDS = %i[invocation_id correlation_id reasoning confidence rationale persona model].freeze

      def retract_edge_facet(edge_handle, property_id, *rest)
        super(edge_handle, property_id, RetractEdgeFacetPositionalActFields.opts_from(rest))
      end

      def retract_edge_facet_with_http_info(edge_handle, property_id, *rest)
        super(edge_handle, property_id, RetractEdgeFacetPositionalActFields.opts_from(rest))
      end

      # `rest` is either the current `[opts]` (or nothing), or the old seven act fields with an
      # optional trailing `opts`.
      def self.opts_from(rest)
        case rest.length
        when 0 then {}
        when 1 then rest.first
        when ACT_FIELDS.length, ACT_FIELDS.length + 1
          legacy = ACT_FIELDS.zip(rest.first(ACT_FIELDS.length)).to_h.compact
          legacy.merge(rest[ACT_FIELDS.length] || {})
        else
          raise ArgumentError,
                "retract_edge_facet takes (edge_handle, property_id, opts = {}); got #{rest.length + 2} arguments"
        end
      end
    end

    Temper::Generated::FacetsApi.prepend(RetractEdgeFacetPositionalActFields)
  end
end
