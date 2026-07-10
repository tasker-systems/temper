# frozen_string_literal: true

RSpec.describe Temper::Client do
  before do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test' }
  end

  after { Temper.reset_connection! }

  let(:bearer) { Temper::Credentials::BearerToken.new('tok-1') }
  # Backoff is injected so the retry specs do not actually sleep.
  let(:no_sleep) { ->(_attempt) {} }

  def client(credentials: bearer)
    described_class.new(credentials: credentials, backoff: no_sleep)
  end

  def m2m_credentials
    Temper::Credentials::ClientCredentials.new(
      token_url: 'https://auth.test/token', client_id: 'c', client_secret: 's', audience: 'a'
    )
  end

  def json(body) = { status: 200, body: body, headers: { 'Content-Type' => 'application/json' } }

  it 'scopes the credential token around the call and stamps the surface header' do
    stub_request(:get, 'https://api.test/api/profile')
      .with(headers: { 'Authorization' => 'Bearer tok-1', 'X-Temper-Surface' => 'sdk' })
      .to_return(json('{"id":"p1"}'))

    client.whoami
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.once
  end

  it 'clears the fiber-local token after the call' do
    stub_request(:get, 'https://api.test/api/profile').to_return(json('{}'))
    client.whoami
    expect(Temper.current_token).to be_nil
  end

  it 'clears the fiber-local token even when the call raises' do
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 404, body: '{}')
    expect { client.whoami }.to raise_error(Temper::NotFound)
    expect(Temper.current_token).to be_nil
  end

  it 'translates a 403 SYSTEM_ACCESS_REQUIRED into the named exception' do
    stub_request(:get, 'https://api.test/api/profile').to_return(
      status: 403,
      body: JSON.generate(error: { code: 'SYSTEM_ACCESS_REQUIRED', message: 'grant the agent cogmap write' })
    )
    expect { client.whoami }
      .to raise_error(Temper::SystemAccessRequired, /grant the agent cogmap write/)
  end

  it 'raises Unauthorized immediately for a BearerToken on 401 -- it cannot refresh' do
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 401, body: '{}')
    expect { client.whoami }.to raise_error(Temper::Unauthorized)
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.once
  end

  # Refresh-ahead-of-expiry is not enough: a Sidekiq job holding a token across a
  # long unit of work outlives it and takes a 401 nothing recovers.
  it 're-mints once and retries when ClientCredentials takes a 401 mid-job' do
    stub_request(:post, 'https://auth.test/token')
      .to_return(json(JSON.generate(access_token: 'tok-a', expires_in: 3600)))
      .then.to_return(json(JSON.generate(access_token: 'tok-b', expires_in: 3600)))

    stub_request(:get, 'https://api.test/api/profile')
      .with(headers: { 'Authorization' => 'Bearer tok-a' })
      .to_return(status: 401, body: '{}')
    stub_request(:get, 'https://api.test/api/profile')
      .with(headers: { 'Authorization' => 'Bearer tok-b' })
      .to_return(json('{"id":"p1"}'))

    expect { client(credentials: m2m_credentials).whoami }.not_to raise_error
    expect(a_request(:post, 'https://auth.test/token')).to have_been_made.twice
  end

  it 'gives up after a single re-mint, rather than looping' do
    stub_request(:post, 'https://auth.test/token')
      .to_return(json(JSON.generate(access_token: 'tok-a', expires_in: 3600)))
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 401, body: '{}')

    expect { client(credentials: m2m_credentials).whoami }.to raise_error(Temper::Unauthorized)
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.twice
  end

  it 'retries an idempotent read on 5xx, three attempts' do
    stub_request(:get, 'https://api.test/api/profile')
      .to_return({ status: 503, body: 'down' }, { status: 503, body: 'down' }, json('{"id":"p1"}'))

    expect { client.whoami }.not_to raise_error
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.times(3)
  end

  it 'raises ServerError after exhausting read retries' do
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 503, body: 'down')
    expect { client.whoami }.to raise_error(Temper::ServerError)
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.times(3)
  end

  it 'does not retry a permanent error on a read' do
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 404, body: '{}')
    expect { client.whoami }.to raise_error(Temper::NotFound)
    expect(a_request(:get, 'https://api.test/api/profile')).to have_been_made.once
  end

  # Same rule as the Rust client's should_retry: safe methods only.
  it 'never auto-retries a write, even on 503' do
    stub_request(:post, 'https://api.test/api/facets').to_return(status: 503, body: 'down')

    expect do
      client.call { |api| api.call_api(:POST, '/api/facets', header_params: {}, body: '{}') }
    end.to raise_error(Temper::ServerError)
    expect(a_request(:post, 'https://api.test/api/facets')).to have_been_made.once
  end

  it 'backs off between read retries' do
    stub_request(:get, 'https://api.test/api/profile').to_return(status: 503, body: 'down')
    attempts = []
    c = described_class.new(credentials: bearer, backoff: ->(n) { attempts << n })
    expect { c.whoami }.to raise_error(Temper::ServerError)
    expect(attempts).to eq([1, 2])
  end
end
