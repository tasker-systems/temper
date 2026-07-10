# frozen_string_literal: true

RSpec.describe Temper do
  it 'exposes a SemVer gem version' do
    expect(Temper::VERSION).to match(/\A\d+\.\d+\.\d+\z/)
  end

  it 'memoizes a single configuration object' do
    first = described_class.config
    expect(described_class.config).to be(first)
  end

  it 'yields the configuration to configure' do
    described_class.configure { |c| c.base_url = 'https://example.test' }
    expect(described_class.config.base_url).to eq('https://example.test')
  end
end
