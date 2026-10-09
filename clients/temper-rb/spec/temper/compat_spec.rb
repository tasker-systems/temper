# frozen_string_literal: true

RSpec.describe Temper::Compat do
  # The old nine-argument form of retract_edge_facet must keep working, and its act fields must
  # now reach the wire as query parameters — the generated core alone raises ArgumentError for it.
  describe 'FacetsApi#retract_edge_facet' do
    let(:api_client) { Temper::Generated::ApiClient.new }
    let(:api) { Temper::Generated::FacetsApi.new(api_client) }
    let(:sent) { {} }

    before do
      allow(api_client).to receive(:call_api) do |_verb, path, opts|
        sent[:path] = path
        sent[:query] = opts[:query_params]
        [nil, 200, {}]
      end
    end

    it 'accepts the current (edge_handle, property_id, opts) form' do
      api.retract_edge_facet('e', 'p', confidence: 'high', reasoning: 'why')
      expect(sent[:path]).to eq('/api/relationships/e/facets/p')
      expect(sent[:query]).to include(confidence: 'high', reasoning: 'why')
    end

    it 'accepts the released nine-argument form and sends its act fields as query parameters' do
      api.retract_edge_facet('e', 'p', 'inv', nil, 'why', 'high', nil, nil, nil)
      expect(sent[:path]).to eq('/api/relationships/e/facets/p')
      expect(sent[:query]).to include(invocation_id: 'inv', reasoning: 'why', confidence: 'high')
      expect(sent[:query]).not_to include(:correlation_id, :rationale, :persona, :model)
    end

    it 'merges a trailing opts hash after the nine-argument form' do
      api.retract_edge_facet_with_http_info('e', 'p', nil, nil, nil, 'low', nil, nil, nil,
                                            { x_temper_surface: 'sdk' })
      expect(sent[:query]).to include(confidence: 'low')
    end

    it 'refuses any other arity with the current signature named' do
      expect { api.retract_edge_facet('e', 'p', 'a', 'b') }
        .to raise_error(ArgumentError, /edge_handle, property_id, opts/)
    end
  end
end
