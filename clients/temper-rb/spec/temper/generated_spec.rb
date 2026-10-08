# frozen_string_literal: true

require 'json'

RSpec.describe Temper::Generated do
  let(:contract) { JSON.parse(File.read(CONTRACT_PATH)) }

  it 'records the contract version it was generated from' do
    expect(Temper::CONTRACT_VERSION).to eq(contract.fetch('info').fetch('version'))
  end

  it 'keeps the gem version independent of the contract version' do
    expect(Temper::VERSION).to match(/\A\d+\.\d+\.\d+\z/)
  end

  # P5 gave every operation a unique operationId. If a future contract change
  # reintroduces a collision, the generator silently emits `list_0` again.
  it 'exposes collision-free resource operations' do
    methods = Temper::Generated::ResourcesApi.instance_methods
    expect(methods).to include(:list_resources, :list_resource_edges)
    expect(methods.grep(/_\d+\z/)).to be_empty
  end

  it 'flattens the seven ActInput keys onto IngestPayload' do
    act_keys = %i[confidence correlation_id invocation_id model persona rationale reasoning]
    expect(Temper::Generated::IngestPayload.instance_methods).to include(*act_keys)
  end

  # A path value of `.` or `..` would climb to the parent route. The generate script escapes `.`
  # at every substitution site and refuses the two dot segments in the one URL builder; both are
  # post-generation patches, so these fail if a regeneration drops either.
  describe 'path values' do
    let(:api_client) { Temper::Generated::ApiClient.new }

    it 'escapes `.` so a dotted value stays one literal segment' do
      seen = nil
      allow(api_client).to receive(:call_api) do |_verb, path, _opts|
        seen = path
        [nil, 200, {}]
      end
      Temper::Generated::SchemaApi.new(api_client).describe_doc_type_with_http_info('a.b')
      expect(seen).to eq('/api/schema/doc-types/a%2Eb')
    end

    ['.', '..', '%2E', '%2E%2E'].each do |segment|
      it "refuses a `#{segment}` segment before a request exists" do
        expect { api_client.build_request_url("/api/schema/doc-types/#{segment}") }
          .to raise_error(ArgumentError, /parent route/)
      end
    end
  end

  it 'exposes the seams the skin depends on' do
    config = Temper::Generated::Configuration.new
    expect(config).to respond_to(:access_token_getter=, :configure_connection)
  end

  # The admission 403 carries a typed refusal. It reaches Ruby as an anonymous `oneOf`, which the
  # generator resolves by trying each branch until one casts — so "it discriminates" is a property
  # of the emitted schema, not something the template guarantees. Keys are symbols because the
  # ApiClient deserializes with `symbolize_names: true`; string keys match no branch at all.
  describe 'the typed refusal' do
    # Named branches, not RefusalOneOf4 — see the `schema(title = …)` note on the Rust enum.
    {
      'no_standing' => 'NoStanding',
      'denied' => 'Denied',
      'requested' => 'Requested',
      'revoked' => 'Revoked',
      'deactivated' => 'Deactivated',
      'no_prior_standing' => 'NoPriorStanding'
    }.each do |kind, klass|
      it "resolves #{kind} to #{klass}" do
        refusal = Temper::Generated::Refusal.build({ kind: kind })
        expect(refusal).to be_a(described_class.const_get(klass))
        expect(refusal.kind).to eq(kind)
      end
    end

    it 'carries the payload of a data-bearing variant' do
      refusal = Temper::Generated::Refusal.build(
        { kind: 'illegal_transition', act: 'approve', from: 'denied' }
      )
      expect(refusal).to be_a(Temper::Generated::IllegalTransition)
      expect(refusal.act).to eq('approve')
      expect(refusal.from).to eq('denied')
    end

    it 'admits a null `from` — an act can be illegal with no standing at all' do
      refusal = Temper::Generated::Refusal.build(
        { kind: 'illegal_transition', act: 'approve', from: nil }
      )
      expect(refusal).to be_a(Temper::Generated::IllegalTransition)
      expect(refusal.from).to be_nil
    end

    it 'distinguishes the two authority sides' do
      refusal = Temper::Generated::Refusal.build(
        { kind: 'insufficient_authority', required: 'admin', actual: 'self_principal' }
      )
      expect(refusal).to be_a(Temper::Generated::InsufficientAuthority)
      expect(refusal.required).to eq('admin')
      expect(refusal.actual).to eq('self_principal')
    end

    # The server may be newer than the gem. Casting an unknown kind into whichever branch happens
    # to match first would report the wrong reason to the operator — worse than reporting none.
    it 'refuses a kind this build does not know rather than mis-casting it' do
      expect(Temper::Generated::Refusal.build({ kind: 'something_new' })).to be_nil
    end

    it 'resolves through the details envelope the 403 actually sends' do
      details = Temper::Generated::SystemAccessDetails.build_from_hash(
        { email: 'p@example.com', display_name: 'Pete', refusal: { kind: 'revoked' },
          request_url: 'https://temperkb.io/request-access', cli_command: 'temper auth request-access' }
      )
      expect(details.refusal).to be_a(Temper::Generated::Revoked)
      expect(details.email).to eq('p@example.com')
    end
  end
end
