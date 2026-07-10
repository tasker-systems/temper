# frozen_string_literal: true

module Temper
  # The resource surface. Returns generated model instances directly -- the skin
  # does not re-wrap them (D14).
  class Resources
    def initialize(client)
      @client = client
    end

    # POST /api/ingest -> 200 IngestCreateResponse.
    #
    # The seven act keys ride as top-level body fields (`#[serde(flatten)] pub
    # act: ActInput`), which the generated IngestPayload already models as plain
    # attributes because the contract expresses it as allOf: [ActInput, {...}].
    #
    # chunks_packed and content_hash are deliberately never sent: both are Option
    # on IngestPayload and the server computes them. Ruby never embeds (D9).
    def create(title:, context_ref:, doc_type_name:, content:, origin_uri: '', act: nil, **opts)
      payload = Generated::IngestPayload.new(
        { title: title, context_ref: context_ref, doc_type_name: doc_type_name,
          content: content, origin_uri: origin_uri }.merge(opts).merge(act_hash(act))
      )
      @client.call { |api| Generated::IngestApi.new(api).create_ingest(payload) }
    end

    # GET /api/resources/{id} takes no query parameters at all, so `edges` is a
    # separate operation rather than a flag, and `meta_only` is a different
    # endpoint entirely.
    def show(ref, meta_only: false)
      id = Temper.parse_ref(ref)
      @client.call(idempotent: true) do |api|
        next Generated::MetaApi.new(api).get_meta(id) if meta_only

        Generated::ResourcesApi.new(api).get_resource(id)
      end
    end

    def edges(ref)
      id = Temper.parse_ref(ref)
      @client.call(idempotent: true) { |api| Generated::ResourcesApi.new(api).list_resource_edges(id) }
    end

    def update(ref, act: nil, **fields)
      id = Temper.parse_ref(ref)
      body = Generated::ResourceUpdateRequest.new(fields.merge(act_hash(act)))
      @client.call { |api| Generated::ResourcesApi.new(api).update_resource(id, body) }
    end

    # DELETE /api/resources/{id} takes Query<ActInput>: the same seven keys, on
    # the query string rather than in a body. The generated delete_resource reads
    # them straight off `opts`, so Act#to_h passes through unchanged.
    def delete(ref, act: nil)
      id = Temper.parse_ref(ref)
      @client.call { |api| Generated::ResourcesApi.new(api).delete_resource(id, act_hash(act)) }
    end

    def list(**filters)
      @client.call(idempotent: true) { |api| Generated::ResourcesApi.new(api).list_resources(filters) }
    end

    private

    def act_hash(act)
      act.nil? ? {} : act.to_h
    end
  end
end
