# frozen_string_literal: true

RSpec.describe Temper::Contexts do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:client) { Temper::Client.new(credentials: Temper::Credentials::BearerToken.new('tok')) }

  def created(body) = { status: 201, body: body, headers: { 'Content-Type' => 'application/json' } }
  def ok(body) = { status: 200, body: body, headers: { 'Content-Type' => 'application/json' } }

  # ContextCreateRequest's fields are `name` and `owner` -- there is no `slug` on
  # the wire; the server derives it.
  #
  # ContextOwnerRef is an externally-tagged serde enum mixing a unit variant with
  # newtype variants, so it has no discriminator and the generated oneOf wrapper
  # is best-effort by its own admission. We hand-build the wire shape.
  it 'hand-constructs the owner ref payload for a personal context' do
    stub_request(:post, 'https://api.test/api/contexts')
      .with { |req| JSON.parse(req.body)['owner'] == 'Me' }
      .to_return(created(Fixtures.context_row_json))

    client.contexts.create(name: 'incidents', owner: :me)
    expect(a_request(:post, 'https://api.test/api/contexts')).to have_been_made.once
  end

  it 'hand-constructs the owner ref payload for a team context' do
    stub_request(:post, 'https://api.test/api/contexts')
      .with { |req| JSON.parse(req.body)['owner'] == { 'Team' => 'acme' } }
      .to_return(created(Fixtures.context_row_json))

    client.contexts.create(name: 'incidents', owner: { team: 'acme' })
    expect(a_request(:post, 'https://api.test/api/contexts')).to have_been_made.once
  end

  it 'hand-constructs the owner ref payload for a profile context' do
    stub_request(:post, 'https://api.test/api/contexts')
      .with { |req| JSON.parse(req.body)['owner'] == { 'Profile' => 'dana' } }
      .to_return(created(Fixtures.context_row_json))

    client.contexts.create(name: 'incidents', owner: { profile: 'dana' })
    expect(a_request(:post, 'https://api.test/api/contexts')).to have_been_made.once
  end

  it 'omits owner entirely when not supplied -- only name is required' do
    stub_request(:post, 'https://api.test/api/contexts')
      .with { |req| !JSON.parse(req.body).key?('owner') }
      .to_return(created(Fixtures.context_row_json))

    client.contexts.create(name: 'incidents')
    expect(a_request(:post, 'https://api.test/api/contexts')).to have_been_made.once
  end

  it 'rejects an owner shape it cannot encode, rather than guessing' do
    expect { client.contexts.create(name: 'x', owner: { squad: 'a' }) }
      .to raise_error(ArgumentError, /team.*profile|profile.*team/)
    expect { client.contexts.create(name: 'x', owner: 'acme') }
      .to raise_error(ArgumentError, /:me/)
  end

  it 'lists contexts as an idempotent read' do
    stub_request(:get, 'https://api.test/api/contexts').to_return(ok('[]'))
    client.contexts.list
    expect(a_request(:get, 'https://api.test/api/contexts')).to have_been_made.once
  end

  it 'shows a context by ref, resolving the trailing UUID' do
    id = '00000000-0000-0000-0003-000000000001'
    stub_request(:get, "https://api.test/api/contexts/#{id}")
      .to_return(ok(Fixtures.context_row_json(id: id)))

    client.contexts.show("temper-#{id}")
    expect(a_request(:get, "https://api.test/api/contexts/#{id}")).to have_been_made.once
  end
end
