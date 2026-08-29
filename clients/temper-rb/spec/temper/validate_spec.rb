# frozen_string_literal: true

RSpec.describe Temper::Validate do
  describe '.require_endpoint' do
    it 'accepts an https origin with a path prefix' do
      expect(described_class.require_endpoint('https://temperkb.io', name: 'base_url')).to be_a(URI)
      expect(described_class.require_endpoint('https://temperkb.io/api', name: 'base_url')).to be_a(URI)
    end

    it 'rejects anything that is not an absolute http(s) URL' do
      ['', 'temperkb.io', '/api/relative', 'ftp://temperkb.io', 'http://'].each do |bad|
        expect { described_class.require_endpoint(bad, name: 'base_url') }
          .to raise_error(ArgumentError), bad
      end
    end

    it 'rejects userinfo because it would ride in every error message' do
      expect { described_class.require_endpoint('https://id:secret@temperkb.io', name: 'base_url') }
        .to raise_error(ArgumentError, /userinfo/)
    end

    it 'rejects a query or fragment that the path join would bury' do
      expect { described_class.require_endpoint('https://temperkb.io?audience=x', name: 'base_url') }
        .to raise_error(ArgumentError, /query or fragment/)
      expect { described_class.require_endpoint('https://temperkb.io#section', name: 'base_url') }
        .to raise_error(ArgumentError, /query or fragment/)
    end

    it 'rejects whitespace rather than letting the parser normalize it' do
      expect { described_class.require_endpoint("https://temperkb.io/\r\nx-auth", name: 'base_url') }
        .to raise_error(ArgumentError, /whitespace/)
      expect { described_class.require_endpoint("https://temper\tkb.io", name: 'base_url') }
        .to raise_error(ArgumentError, /whitespace/)
    end

    it 'rejects an unparseable port' do
      expect { described_class.require_endpoint('https://temperkb.io:99999', name: 'base_url') }
        .to raise_error(ArgumentError)
    end

    it 'allows plaintext to the loopback interface' do
      ['http://localhost', 'http://localhost:3000', 'http://127.0.0.1',
       'http://127.255.42.42:8123', # the whole 127.0.0.0/8 block, not just .0.0.1
       'http://[::1]:0', 'http://worker.localhost'].each do |url|
        expect { described_class.require_endpoint(url, name: 'base_url') }
          .not_to raise_error, url
      end
    end

    it 'refuses plaintext to anything else' do
      ['http://temperkb.io', 'http://10.0.0.5:8080', 'http://192.168.1.10'].each do |url|
        expect { described_class.require_endpoint(url, name: 'base_url') }
          .to raise_error(ArgumentError, /non-loopback/), url
      end
    end

    it 'the opt-out is a keyword the caller has to write' do
      expect do
        described_class.require_endpoint('http://temperkb.io', name: 'base_url',
                                                          allow_insecure_http: true)
      end.not_to raise_error
    end
  end

  describe '.loopback?' do
    it 'names this machine by literal address or reserved name' do
      expect(described_class.loopback?('localhost')).to be(true)
      expect(described_class.loopback?('LOCALHOST')).to be(true) # URI#host is lowercased; direct calls are not
      expect(described_class.loopback?('localhost.')).to be(true) # one fully-qualified trailing dot
      expect(described_class.loopback?('app.localhost')).to be(true)
      expect(described_class.loopback?('127.0.0.1')).to be(true)
      expect(described_class.loopback?('127.9.9.9')).to be(true)
      expect(described_class.loopback?('::1')).to be(true)
      expect(described_class.loopback?('[::1]')).to be(true) # URI#host keeps the brackets
      expect(described_class.loopback?('temperkb.io')).to be(false)
      expect(described_class.loopback?('10.0.0.1')).to be(false)
      expect(described_class.loopback?('localhost.example.com')).to be(false) # suffix of a longer name
      expect(described_class.loopback?('127.0.0.2.example.com')).to be(false)
    end
  end
end

RSpec.describe 'the seams' do
  describe 'Temper.api_client' do
    after { Temper.reset_connection! }

    it 'refuses a plaintext non-loopback base_url where the client is built' do
      Temper.reset_connection!
      Temper.configure { |c| c.base_url = 'http://api.test' }
      expect { Temper.api_client }.to raise_error(ArgumentError, /base_url is plaintext http/)
    end

    it 'allows it when configuration opts in deliberately' do
      Temper.reset_connection!
      Temper.configure do |c|
        c.base_url = 'http://api.test'
        c.allow_insecure_http = true
      end
      expect { Temper.api_client }.not_to raise_error
    end
  end

  describe Temper::Credentials::ClientCredentials do
    it 'refuses a plaintext non-loopback token URL at construction' do
      expect do
        described_class.new(token_url: 'http://idp.example.com/oauth/token',
                            client_id: 'cid', client_secret: 'sec')
      end.to raise_error(ArgumentError, /token_url is plaintext http/)
    end

    it 'accepts a loopback token URL without the opt-in' do
      expect do
        described_class.new(token_url: 'http://127.0.0.1:9999/oauth/token',
                            client_id: 'cid', client_secret: 'sec')
      end.not_to raise_error
    end
  end
end
