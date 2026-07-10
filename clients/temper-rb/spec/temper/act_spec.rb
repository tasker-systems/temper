# frozen_string_literal: true

RSpec.describe Temper::Act do
  it 'accepts confidence alone' do
    expect(described_class.new(confidence: :probable).to_h).to eq(confidence: 'probable')
  end

  it 'accepts an empty act' do
    expect(described_class.new.to_h).to eq({})
  end

  # ActInput::into_act_context builds an AgentAuthorship whose `confidence` is
  # non-Option. Supplying any authorship field without it earns a 400; we reject
  # it locally instead of paying the round trip.
  %i[reasoning rationale persona model].each do |field|
    it "rejects #{field} without confidence, locally, rather than earning a 400" do
      expect { described_class.new(field => 'x') }.to raise_error(ArgumentError, /confidence/)
    end

    it "accepts #{field} alongside confidence" do
      act = described_class.new(:confidence => :certain, field => 'x')
      expect(act.to_h[field]).to eq('x')
    end
  end

  it 'names every missing authorship field in the error' do
    expect { described_class.new(reasoning: 'r', persona: 'p') }
      .to raise_error(ArgumentError, /reasoning.*persona|persona.*reasoning/)
  end

  it 'renames correlation and invocation to their wire keys' do
    act = described_class.new(correlation: 'c-1', invocation: 'i-1')
    expect(act.to_h).to eq(correlation_id: 'c-1', invocation_id: 'i-1')
  end

  it 'omits nils entirely so the server sees an absent key, not null' do
    expect(described_class.new(confidence: :probable).to_h.keys).to eq([:confidence])
  end

  it 'stringifies a symbol confidence' do
    expect(described_class.new(confidence: :speculative).to_h[:confidence]).to eq('speculative')
  end

  # Correlation is provenance, never authorship. An act with no supplied
  # correlation self-roots to its own event id, so it is exempt from the invariant.
  it 'permits correlation with no confidence' do
    expect { described_class.new(correlation: 'c-1') }.not_to raise_error
  end

  it 'permits invocation with no confidence' do
    expect { described_class.new(invocation: 'i-1') }.not_to raise_error
  end

  it 'returns a fresh hash each call, so callers cannot mutate the act' do
    act = described_class.new(confidence: :certain)
    act.to_h[:confidence] = 'tampered'
    expect(act.to_h[:confidence]).to eq('certain')
  end

  it 'carries all seven ActInput wire keys when fully populated' do
    act = described_class.new(confidence: :certain, reasoning: 'r', rationale: 'ra',
                              persona: 'p', model: 'm', correlation: 'c', invocation: 'i')
    expect(act.to_h.keys).to contain_exactly(:confidence, :reasoning, :rationale, :persona,
                                             :model, :correlation_id, :invocation_id)
  end
end
