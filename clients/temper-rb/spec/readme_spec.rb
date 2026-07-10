# frozen_string_literal: true

# A doc that drifts from the code is worse than no doc. These pin the three
# claims a user acts on and cannot verify from the contract.
RSpec.describe 'README' do
  let(:readme) { File.read(File.expand_path('../README.md', __dir__)) }

  it 'documents the four TEMPER_M2M_* variables by their real names' do
    %w[TEMPER_M2M_TOKEN_URL TEMPER_M2M_CLIENT_ID TEMPER_M2M_CLIENT_SECRET TEMPER_M2M_AUDIENCE]
      .each { |var| expect(readme).to include(var) }
  end

  # Authentication is not authorization: a minted M2M token yields a
  # JIT-provisioned agent profile and nothing else.
  it 'carries a Going live section naming both authorization steps' do
    expect(readme).to match(/##\s*Going live/i)
    expect(readme).to match(/cogmap write grant/i)
    expect(readme).to match(/team membership/i)
  end

  it 'documents the fork-safety hooks' do
    expect(readme).to include('Temper.reset_connection!')
    expect(readme).to include('on_worker_boot')
    expect(readme).to include('Sidekiq.configure_server')
  end

  it 'names the methods it demonstrates' do
    expect(readme).to include('Temper::Credentials::BearerToken')
    expect(readme).to include('Temper::Credentials::ClientCredentials')
    expect(readme).to include('whoami')
  end

  it 'explains why bulk reconcile is absent' do
    expect(readme).to match(/reconcile/i)
    expect(readme).to include('chunks_packed')
  end
end
