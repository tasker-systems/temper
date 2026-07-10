# frozen_string_literal: true

RSpec.describe Temper::CognitiveMaps do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:client) { Temper::Client.new(credentials: Temper::Credentials::BearerToken.new('tok')) }
  let(:cogmap_id) { '00000000-0000-0000-0005-000000000001' }
  let(:other) { '019f4bdc-4cd2-73f2-a866-4cc29606de66' }

  def json(body) = { status: 200, body: body, headers: { 'Content-Type' => 'application/json' } }

  def genesis_body
    JSON.generate(cogmap_id: cogmap_id, telos_resource_id: other, created: true)
  end

  # Genesis takes an Option<ReconcileTelos>, so a charter-less map is creatable --
  # but CreateCogmapRequest still requires `telos_title`. Charter-less is not
  # title-less.
  it 'creates a charter-less map, sending telos_title but no telos' do
    stub_request(:post, 'https://api.test/api/cognitive-maps')
      .with do |req|
        body = JSON.parse(req.body)
        body['name'] == 'team-self' && body['telos_title'] == 'Purpose' && !body.key?('telos')
      end
      .to_return(json(genesis_body))

    client.cognitive_maps.create(name: 'team-self', telos_title: 'Purpose')
    expect(a_request(:post, 'https://api.test/api/cognitive-maps')).to have_been_made.once
  end

  it 'requires telos_title, per the contract' do
    expect { client.cognitive_maps.create(name: 'x') }.to raise_error(ArgumentError, /telos_title/)
  end

  it 'authors into a map via ingest with home_cogmap_id -- the server embeds' do
    stub_request(:post, 'https://api.test/api/ingest')
      .with do |req|
        body = JSON.parse(req.body)
        body['home_cogmap_id'] == cogmap_id && !body.key?('chunks_packed')
      end
      .to_return(json('{}'))

    client.cognitive_maps.author(cogmap_id, title: 'Node', content: '# n',
                                            context_ref: '@d/c', doc_type_name: 'note')
    expect(a_request(:post, 'https://api.test/api/ingest')).to have_been_made.once
  end

  # POST /api/relationships -- NOT /api/relationships/assert, which the design
  # spec names and which does not exist. The seven act keys flatten on.
  it 'asserts an edge against /api/relationships with flattened act keys' do
    stub_request(:post, 'https://api.test/api/relationships')
      .with do |req|
        body = JSON.parse(req.body)
        body['source'] == cogmap_id && body['target'] == other &&
          body['edge_kind'] == 'leads_to' && body['polarity'] == 'forward' &&
          body['label'] == 'advances' && (body['weight'] - 1.0).abs < Float::EPSILON &&
          body['confidence'] == 'probable'
      end
      .to_return(json('{"edge_handle":"eh-1"}'))

    client.cognitive_maps.assert_relationship(
      source: cogmap_id, target: other, edge_kind: 'leads_to', label: 'advances', weight: 1.0,
      act: Temper::Act.new(confidence: :probable)
    )
    expect(a_request(:post, 'https://api.test/api/relationships')).to have_been_made.once
  end

  it 'defaults polarity to forward' do
    stub_request(:post, 'https://api.test/api/relationships')
      .with { |req| JSON.parse(req.body)['polarity'] == 'forward' }
      .to_return(json('{"edge_handle":"eh-1"}'))

    client.cognitive_maps.assert_relationship(source: cogmap_id, target: other,
                                              edge_kind: 'near', label: 'rel', weight: 0.5)
    expect(a_request(:post, 'https://api.test/api/relationships')).to have_been_made.once
  end

  it 'resolves decorated refs on both edge endpoints' do
    stub_request(:post, 'https://api.test/api/relationships')
      .with { |req| JSON.parse(req.body).values_at('source', 'target') == [cogmap_id, other] }
      .to_return(json('{"edge_handle":"eh-1"}'))

    client.cognitive_maps.assert_relationship(source: "map-#{cogmap_id}", target: "task-#{other}",
                                              edge_kind: 'near', label: 'rel', weight: 0.5)
    expect(a_request(:post, 'https://api.test/api/relationships')).to have_been_made.once
  end

  # POST /api/facets -- NOT /api/facets/set. `values` is plural.
  it 'sets a facet against /api/facets' do
    stub_request(:post, 'https://api.test/api/facets')
      .with { |req| JSON.parse(req.body)['values'] == { 'tier' => 'core' } }
      .to_return(json(JSON.generate(id: other, property_id: cogmap_id)))

    client.cognitive_maps.set_facet(resource: cogmap_id, values: { 'tier' => 'core' })
    expect(a_request(:post, 'https://api.test/api/facets')).to have_been_made.once
  end

  # D9: reconcile requires a client-side 768-dim BGE embedder, which Ruby has not
  # and never will. The generated method stays reachable for anyone hand-packing
  # chunks; this is the guard against someone surfacing it on the skin.
  it 'does not surface bulk reconcile' do
    expect(client.cognitive_maps).not_to respond_to(:reconcile)
  end

  it 'still reaches reconcile through the generated API, deliberately' do
    expect(Temper::Generated::CognitiveMapsApi.instance_methods).to include(:reconcile)
  end
end
