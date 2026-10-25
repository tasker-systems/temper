# frozen_string_literal: true

RSpec.describe Temper::WireTime do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:api) { Temper::Generated::DataArtifactsApi.new(Temper.api_client) }
  let(:shape) { '019f4912-3f20-7fd3-814f-13a5ddbe3cd7' }
  let(:artifact_id) { '019f4912-3f20-7fd3-814f-13a5ddbe3cd8' }
  # A position as the server writes it: RFC 3339, UTC, to the microsecond.
  let(:created) { '2026-10-25T09:58:12.123456Z' }

  def json(body) = { status: 200, body: JSON.generate(body), headers: { 'Content-Type' => 'application/json' } }

  def page(next_position)
    {
      shape: { shape_id: shape, shape_version: 1, home_anchor_table: 'kb_contexts', home_anchor_id: shape },
      artifacts: [],
      oversized: [],
      population: nil,
      ordered_by: 'created',
      next: next_position
    }
  end

  def ask(after = nil)
    request = Temper::Generated::AskRequest.new(ask: Temper::Generated::ArtifactAsk.new(shape: shape), after: after)
    Temper.with_token('tok') { api.ask_data_artifacts(request) }
  end

  # The client parses a page's `next` into a Time. Sent back as `after`, it must name the row the
  # server named: the same text, sub-seconds included, not `Time#to_s`.
  it "sends a page's `next` back as `after` in the text the server wrote it in" do
    url = 'https://api.test/api/data-artifacts/ask'
    stub_request(:post, url).to_return(json(page({ artifact_id: artifact_id, created: created })))

    first = ask
    expect(first._next.created).to be_a(Time)
    ask(first._next)

    position = { 'artifact_id' => artifact_id, 'created' => created }
    expect(a_request(:post, url).with { |req| JSON.parse(req.body)['after'] == position }).to have_been_made.once
  end

  # The other request-side timestamp in the contract: `after_created` on a bounded artifact list.
  it 'writes a Time query parameter as RFC 3339 to the microsecond' do
    url = %r{\Ahttps://api\.test/api/resources/#{shape}/artifacts}
    stub_request(:get, url).to_return(json([]))

    Temper.with_token('tok') do
      api.list_artifacts(shape, limit: 1, after_created: Time.parse(created), after_id: artifact_id)
    end

    expect(a_request(:get, url).with(query: hash_including('after_created' => created))).to have_been_made.once
  end

  it 'stamps a Time or a DateTime, and leaves everything else as it is' do
    at = Time.parse(created)
    expect(described_class.stamp(at)).to eq(created)
    expect(described_class.stamp(at.to_datetime)).to eq('2026-10-25T09:58:12.123456+00:00')
    holding = { a: [at] }
    ['x', 1, nil, holding].each { |other| expect(described_class.stamp(other)).to equal(other) }
  end

  # A position kept as plain data (`page._next.to_hash`, a `Time` inside) and passed back as a hash
  # in the cursor's place is still the declared `ArtifactCursor`: its `created` is written to the
  # microsecond, whichever key spelling the caller used, and the caller's hash is left unchanged.
  it 'writes a declared timestamp in a part the caller passes as a hash' do
    at = Time.parse(created)
    [{ created: at, artifact_id: artifact_id }, { 'created' => at, 'artifact_id' => artifact_id }].each do |after|
      request = Temper::Generated::AskRequest.new(ask: { shape: shape }, after: after)
      body = JSON.parse(Temper.api_client.object_to_http_body(request))
      expect(body['after']['created']).to eq(created)
      expect(after.values).to include(at)
    end
  end

  # A `Time` in free-form JSON is no declared timestamp: the body is exactly the one the generated
  # core writes, so the content a caller commits is stored, and hashed, as the caller wrote it. A
  # generated model placed there is free-form too: its declared timestamps are not stamped.
  it 'leaves a Time in free-form content to the json gem' do
    at = Time.parse(created)
    model = Temper::Generated::ArtifactCursor.new(created: at, artifact_id: artifact_id)
    [at, { observed_at: at, readings: [at, { at: at }] }, model, [model]].each do |content|
      request = Temper::Generated::ArtifactCommitRequest.new(kind: 'measurement', intent: 'current', content: content)
      expect(Temper.api_client.object_to_http_body(request)).to eq(request.to_hash.to_json)
    end
  end

  # The other free-form spellings: a map (`Hash<String, Object>`, a facet's `values`), whether it
  # holds a generated model or is one, and an untyped array (`Array<Object>`, a contains op's
  # `values`) holding one are written exactly as the generated core writes them. And a oneOf slot
  # (`PropertyOp`) is not entered, whichever variant it holds.
  it 'leaves a model in a free-form map, an untyped array or a oneOf slot to the json gem' do
    model = Temper::Generated::ArtifactCursor.new(created: Time.parse(created), artifact_id: artifact_id)
    in_map = Temper::Generated::FacetSetRequest.new(resource: shape, values: { 'at' => model })
    as_map = Temper::Generated::FacetSetRequest.new(resource: shape, values: {}).tap { |r| r.values = model }
    op = Temper::Generated::PropertyOpOneOf1.new(op: 'contains', values: [model])
    in_one_of = Temper::Generated::ArtifactPredicate.new(path: ['at'], op: op)
    [in_map, as_map, op, in_one_of].each do |request|
      expect(Temper.api_client.object_to_http_body(request)).to eq(request.to_hash.to_json)
    end
  end
end
