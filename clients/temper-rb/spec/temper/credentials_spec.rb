# frozen_string_literal: true

RSpec.describe Temper::Credentials do
  describe Temper::Credentials::BearerToken do
    it 'returns the token it was constructed with, with no I/O' do
      expect(described_class.new('abc').token).to eq('abc')
    end

    it 'rejects an empty token at construction' do
      expect { described_class.new('') }.to raise_error(ArgumentError)
    end

    it 'rejects a non-String token at construction' do
      expect { described_class.new(nil) }.to raise_error(ArgumentError)
    end

    it 'cannot refresh and says so' do
      expect { described_class.new('abc').refresh! }.to raise_error(Temper::Unauthorized)
    end
  end

  describe Temper::Credentials::ClientCredentials do
    let(:token_url) { 'https://auth.test/oauth/token' }
    let(:base) { Time.at(1_000_000) }

    def creds(clock = -> { base })
      described_class.new(token_url: token_url, client_id: 'cid', client_secret: 'sec',
                          audience: 'https://api.test', clock: clock)
    end

    def stub_mint(token, expires_in: 3600)
      stub_request(:post, token_url)
        .to_return(status: 200, body: JSON.generate(access_token: token, expires_in: expires_in),
                   headers: { 'Content-Type' => 'application/json' })
    end

    it 'mints a token on first use and sends the four M2M fields' do
      stub_mint('tok-1')
      expect(creds.token).to eq('tok-1')

      expect(a_request(:post, token_url).with do |req|
        body = JSON.parse(req.body)
        body['grant_type'] == 'client_credentials' && body['client_id'] == 'cid' &&
          body['client_secret'] == 'sec' && body['audience'] == 'https://api.test'
      end).to have_been_made.once
    end

    it 'caches the token across calls' do
      stub_mint('tok-1')
      c = creds
      3.times { c.token }
      expect(a_request(:post, token_url)).to have_been_made.once
    end

    # The cache is keyed on an ABSOLUTE expires_at with a 60s skew, exactly as
    # the steward's mintM2mToken does. Refresh-ahead-of-expiry, not at it.
    it 'refreshes 60s before the absolute expiry, not at it' do
      now = base
      c = creds(-> { now })

      stub_mint('tok-1', expires_in: 3600)
      expect(c.token).to eq('tok-1')

      stub_mint('tok-2')
      now = base + 3600 - 61   # 61s of headroom: still outside the skew window
      expect(c.token).to eq('tok-1')

      now = base + 3600 - 59   # 59s of headroom: inside the 60s skew, must re-mint
      expect(c.token).to eq('tok-2')
    end

    it 'raises Unauthorized when the mint is rejected' do
      stub_request(:post, token_url).to_return(status: 401, body: 'bad client')
      expect { creds.token }.to raise_error(Temper::Unauthorized, /401/)
    end

    it 'refresh! mints unconditionally, even on a warm cache' do
      stub_mint('tok-1')
      c = creds
      c.token
      stub_mint('tok-2')
      expect(c.refresh!).to eq('tok-2')
    end

    it 'requires every M2M field, throwing rather than defaulting' do
      expect do
        described_class.new(token_url: token_url, client_id: '', client_secret: 's', audience: 'a')
      end.to raise_error(ArgumentError, /client_id/)

      expect do
        described_class.new(token_url: token_url, client_id: 'c', client_secret: 's', audience: nil)
      end.to raise_error(ArgumentError, /audience/)
    end

    # The steward's cache is a bare module global -- sound only because a
    # serverless function is single-threaded. Puma is not.
    it 'mints once when many threads race a cold cache' do
      stub_mint('tok-1')
      c = creds
      threads = Array.new(8) { Thread.new { c.token } }
      expect(threads.map(&:value).uniq).to eq(['tok-1'])
      expect(a_request(:post, token_url)).to have_been_made.once
    end
  end
end
