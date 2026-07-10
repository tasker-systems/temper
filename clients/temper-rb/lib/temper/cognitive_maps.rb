# frozen_string_literal: true

module Temper
  # Incremental cognitive-map authoring (D9).
  #
  # Every path here is server-recompute: the server chunks and embeds. Bulk
  # reconcile (PUT /api/cognitive-maps/{id}) is deliberately absent -- its
  # ReconcileEntry.chunks_packed is a required, PRE-EMBEDDED String, carried
  # verbatim with no server-side ONNX fallback, so a Ruby client cannot physically
  # reach it without a 768-dim BGE embedder. It is a CLI operator's job, not a
  # Rails request's. Reach Generated::CognitiveMapsApi#reconcile directly if you
  # hand-pack chunks.
  class CognitiveMaps
    DEFAULT_POLARITY = 'forward'

    def initialize(client)
      @client = client
    end

    # POST /api/cognitive-maps -> 200 CreateCogmapOutcome.
    #
    # Genesis takes an Option<ReconcileTelos>, so a charter-less map is creatable
    # -- but CreateCogmapRequest still requires `telos_title`. Charter-less is not
    # title-less. CreateCogmapRequest carries no act keys.
    def create(name:, telos_title:, telos: nil, cogmap_id: nil, telos_resource_id: nil)
      attrs = { name: name, telos_title: telos_title }
      attrs[:telos] = telos unless telos.nil?
      attrs[:cogmap_id] = cogmap_id unless cogmap_id.nil?
      attrs[:telos_resource_id] = telos_resource_id unless telos_resource_id.nil?

      body = Generated::CreateCogmapRequest.new(attrs)
      @client.call { |api| Generated::CognitiveMapsApi.new(api).genesis(body) }
    end

    # A map node is a NEW resource that distills from its sources; ingest with
    # home_cogmap_id is how it is born. The server chunks and embeds.
    def author(cogmap_ref, title:, content:, context_ref:, doc_type_name:, act: nil, **opts)
      @client.resources.create(
        title: title, context_ref: context_ref, doc_type_name: doc_type_name,
        content: content, act: act, home_cogmap_id: Temper.parse_ref(cogmap_ref), **opts
      )
    end

    # POST /api/relationships (not /api/relationships/assert). The contract requires
    # all of source, target, edge_kind, polarity, label, weight; the seven act keys
    # flatten on, as they do on IngestPayload.
    #
    # edge_kind is one of express, contains, leads_to, near.
    def assert_relationship(source:, target:, edge_kind:, label:, weight:,
                            polarity: DEFAULT_POLARITY, act: nil)
      body = Generated::AssertRelationshipRequest.new(
        { source: Temper.parse_ref(source), target: Temper.parse_ref(target),
          edge_kind: edge_kind, polarity: polarity, label: label, weight: weight }
          .merge(act_hash(act))
      )
      @client.call { |api| Generated::RelationshipsApi.new(api).assert(body) }
    end

    # POST /api/facets (not /api/facets/set). `values` is plural, and is the
    # facet's typed value payload.
    def set_facet(resource:, values:, weight: nil, act: nil)
      attrs = { resource: Temper.parse_ref(resource), values: values }
      attrs[:weight] = weight unless weight.nil?

      body = Generated::FacetSetRequest.new(attrs.merge(act_hash(act)))
      @client.call { |api| Generated::FacetsApi.new(api).set_facet(body) }
    end

    private

    def act_hash(act)
      act.nil? ? {} : act.to_h
    end
  end
end
