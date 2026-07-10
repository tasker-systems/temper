# frozen_string_literal: true

RSpec.describe 'Temper connection' do
  before do
    Temper.reset_connection!
    Temper.configure do |c|
      c.base_url = 'https://api.test'
      c.device_id = nil
    end
  end

  after { Temper.reset_connection! }

  it 'memoizes one ApiClient per process' do
    first = Temper.api_client
    expect(Temper.api_client).to be(first)
  end

  it 'reset_connection! drops the memo so a forked worker builds fresh sockets' do
    first = Temper.api_client
    Temper.reset_connection!
    expect(Temper.api_client).not_to be(first)
  end

  it 'raises when base_url is unset rather than building a useless client' do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = nil }
    expect { Temper.api_client }.to raise_error(ArgumentError, /base_url/)
  end

  it 'stamps X-Temper-Surface: sdk once, on the client' do
    expect(Temper.api_client.default_headers['X-Temper-Surface']).to eq('sdk')
  end

  it 'omits the device header when unconfigured' do
    expect(Temper.api_client.default_headers).not_to have_key('X-Temper-Device-Id')
  end

  it 'sends the device header when configured' do
    Temper.reset_connection!
    Temper.configure { |c| c.device_id = 'dev-1' }
    expect(Temper.api_client.default_headers['X-Temper-Device-Id']).to eq('dev-1')
  end

  it 'derives scheme, host, and base_path from base_url' do
    Temper.reset_connection!
    Temper.configure { |c| c.base_url = 'https://api.test:8443/prefix' }
    config = Temper.api_client.config
    expect(config.scheme).to eq('https')
    expect(config.host).to eq('api.test:8443')
    expect(config.base_path).to eq('/prefix')
  end

  it 'omits a default port from the host' do
    expect(Temper.api_client.config.host).to eq('api.test')
  end

  # D11: the generated build_connection sets Faraday.default_adapter BETWEEN
  # configure_middleware and configure_connection. Swapping the adapter in the
  # former is silently overwritten; only the latter wins. A silent fallback to
  # net_http costs a TLS handshake per request and nothing fails.
  it 'installs the persistent adapter, not faraday default' do
    conn = Temper.api_client.send(:build_connection) { nil }
    expect(conn.builder.adapter).to eq(Faraday::Adapter::NetHttpPersistent)
  end

  # This is the whole point of D12: the connection is shared, the TOKEN is not.
  it 'resolves the access token per call, from fiber-local storage' do
    getter = Temper.api_client.config.access_token_getter
    Temper.with_token('tok-a') { expect(getter.call).to eq('tok-a') }
    Temper.with_token('tok-b') { expect(getter.call).to eq('tok-b') }
    expect(getter.call).to be_nil
  end

  it 'never leaks a token into a spawned thread' do
    seen = Queue.new
    Temper.with_token('outer') do
      Thread.new { seen << Temper.current_token }.join
    end
    expect(seen.pop).to be_nil
  end

  it 'restores the previous token after a nested scope' do
    Temper.with_token('outer') do
      Temper.with_token('inner') { nil }
      expect(Temper.current_token).to eq('outer')
    end
  end

  it 'clears the token even when the block raises' do
    expect { Temper.with_token('t') { raise 'boom' } }.to raise_error('boom')
    expect(Temper.current_token).to be_nil
  end

  it 'never touches the generated singletons' do
    Temper.api_client
    expect(Temper::Generated::Configuration.default.access_token).to be_nil
  end
end
