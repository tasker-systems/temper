# frozen_string_literal: true

RSpec.describe Temper::Resources do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:client) { Temper::Client.new(credentials: Temper::Credentials::BearerToken.new('tok')) }
  let(:uuid) { '019f4912-3f20-7fd3-814f-13a5ddbe3cd7' }
  let(:act) { Temper::Act.new(confidence: :probable, reasoning: 'because', correlation: 'corr-1') }

  def json(body = '{}') = { status: 200, body: body, headers: { 'Content-Type' => 'application/json' } }

  def row_json = Fixtures.resource_row_json(id: uuid)

  # ~30 write endpoints take act context via `#[serde(flatten)] pub act: ActInput`,
  # and the contract models IngestPayload as allOf: [ActInput, {...}] -- which the
  # generator flattens into plain attributes.
  it 'flattens the act keys into the ingest body' do
    stub_request(:post, 'https://api.test/api/ingest')
      .with do |req|
        body = JSON.parse(req.body)
        body['confidence'] == 'probable' && body['reasoning'] == 'because' &&
          body['correlation_id'] == 'corr-1' && body['title'] == 'Postmortem'
      end
      .to_return(json)

    client.resources.create(title: 'Postmortem', context_ref: '@dana/incidents',
                            doc_type_name: 'note', content: '# hi', act: act)
    expect(a_request(:post, 'https://api.test/api/ingest')).to have_been_made.once
  end

  # Ruby has no BGE embedder and never will (D9). Both fields are Option on
  # IngestPayload and the server computes them.
  it 'never sends chunks_packed or content_hash -- the server computes them' do
    stub_request(:post, 'https://api.test/api/ingest')
      .with do |req|
        body = JSON.parse(req.body)
        !body.key?('chunks_packed') && !body.key?('content_hash')
      end
      .to_return(json)

    client.resources.create(title: 'T', context_ref: '@d/c', doc_type_name: 'note', content: 'x')
    expect(a_request(:post, 'https://api.test/api/ingest')).to have_been_made.once
  end

  it 'passes through extra ingest attributes such as home_cogmap_id' do
    stub_request(:post, 'https://api.test/api/ingest')
      .with { |req| JSON.parse(req.body)['home_cogmap_id'] == uuid }
      .to_return(json)

    client.resources.create(title: 'T', context_ref: '@d/c', doc_type_name: 'note',
                            content: 'x', home_cogmap_id: uuid)
    expect(a_request(:post, 'https://api.test/api/ingest')).to have_been_made.once
  end

  # The generated models raise on an unknown attribute, so a skin typo fails at
  # construction rather than silently dropping a field on the floor.
  it 'raises on an unknown ingest attribute rather than dropping it' do
    expect do
      client.resources.create(title: 'T', context_ref: '@d/c', doc_type_name: 'note',
                              content: 'x', not_a_field: 1)
    end.to raise_error(ArgumentError, /not_a_field/)
  end

  # DELETE /api/resources/{id} takes Query<ActInput>, not a body. Getting this
  # backwards silently drops provenance instead of erroring.
  it 'routes the act keys onto the query string for delete' do
    stub_request(:delete, "https://api.test/api/resources/#{uuid}")
      .with(query: hash_including('confidence' => 'probable', 'correlation_id' => 'corr-1',
                                  'reasoning' => 'because'))
      .to_return(json('{"deleted":true}'))

    client.resources.delete("some-slug-#{uuid}", act: act)
    expect(a_request(:delete, %r{https://api\.test/api/resources/#{uuid}})).to have_been_made.once
  end

  it 'sends no query string when deleting without an act' do
    stub_request(:delete, "https://api.test/api/resources/#{uuid}").to_return(json('{"deleted":true}'))
    client.resources.delete(uuid)
    expect(a_request(:delete, "https://api.test/api/resources/#{uuid}")).to have_been_made.once
  end

  it 'resolves a decorated ref to its trailing UUID before addressing' do
    stub_request(:get, "https://api.test/api/resources/#{uuid}").to_return(json(row_json))
    client.resources.show("stale-wrong-slug-#{uuid}")
    expect(a_request(:get, "https://api.test/api/resources/#{uuid}")).to have_been_made.once
  end

  it 'returns a validated generated model, not a bare Hash' do
    stub_request(:get, "https://api.test/api/resources/#{uuid}").to_return(json(row_json))
    detail = client.resources.show(uuid)
    expect(detail).to be_a(Temper::Generated::ResourceDetail)
    expect(detail.title).to eq('A Resource')
  end

  # ResourceDetail is allOf: [ResourceRow, {...}] and the generator flattens
  # ResourceRow's ten required fields onto it, validating each on deserialize.
  it 'raises when the server omits a required field' do
    stub_request(:get, "https://api.test/api/resources/#{uuid}").to_return(json('{"id":"x"}'))
    expect { client.resources.show(uuid) }.to raise_error(ArgumentError, /cannot be nil/)
  end

  it 'rejects a ref with no trailing UUID before making a request' do
    expect { client.resources.show('just-a-slug') }.to raise_error(ArgumentError)
    expect(a_request(:get, %r{https://api\.test/api/resources/.*})).not_to have_been_made
  end

  it 'reads the meta projection without the body' do
    stub_request(:get, "https://api.test/api/resources/#{uuid}/meta")
      .to_return(json(JSON.generate(id: uuid)))
    client.resources.show(uuid, meta_only: true)
    expect(a_request(:get, "https://api.test/api/resources/#{uuid}/meta")).to have_been_made.once
  end

  # GET /api/resources/{id} has NO query parameters, so edges is its own operation.
  it 'reads edges through the dedicated operation, not a flag on show' do
    stub_request(:get, "https://api.test/api/resources/#{uuid}/edges").to_return(json('[]'))
    client.resources.edges(uuid)
    expect(a_request(:get, "https://api.test/api/resources/#{uuid}/edges")).to have_been_made.once
  end

  it 'updates with a partial body carrying the act keys' do
    stub_request(:patch, "https://api.test/api/resources/#{uuid}")
      .with do |req|
        body = JSON.parse(req.body)
        body['title'] == 'New' && body['confidence'] == 'probable' && !body.key?('content')
      end
      .to_return(json(row_json))

    client.resources.update(uuid, title: 'New', act: act)
    expect(a_request(:patch, "https://api.test/api/resources/#{uuid}")).to have_been_made.once
  end

  it 'lists with filters as an idempotent read' do
    stub_request(:get, 'https://api.test/api/resources')
      .with(query: hash_including('doc_type_name' => 'task', 'stage' => 'in-progress'))
      .to_return(json('[]'))

    client.resources.list(doc_type_name: 'task', stage: 'in-progress')
    expect(a_request(:get, %r{https://api\.test/api/resources\?})).to have_been_made.once
  end

  it 'memoizes the surface on the client' do
    memoized = client
    first = memoized.resources
    expect(memoized.resources).to be(first)
  end
end
