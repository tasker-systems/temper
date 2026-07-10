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

  it 'exposes the seams the skin depends on' do
    config = Temper::Generated::Configuration.new
    expect(config).to respond_to(:access_token_getter=, :configure_connection)
  end
end
