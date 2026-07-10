# frozen_string_literal: true

RSpec.describe 'Temper.parse_ref' do
  let(:uuid) { '019f4912-3f20-7fd3-814f-13a5ddbe3cd7' }

  it 'accepts a bare UUID' do
    expect(Temper.parse_ref(uuid)).to eq(uuid)
  end

  it 'accepts a decorated ref and ignores the slug half' do
    expect(Temper.parse_ref("p4-design-the-temper-rb-gem-#{uuid}")).to eq(uuid)
  end

  # Resolution is trailing-UUID-only: a stale slug half is harmless.
  it 'ignores a stale slug half entirely' do
    expect(Temper.parse_ref("totally-wrong-slug-#{uuid}")).to eq(uuid)
  end

  it 'accepts a slug whose words look like hex' do
    expect(Temper.parse_ref("beef-cafe-#{uuid}")).to eq(uuid)
  end

  it 'accepts a single-word slug' do
    expect(Temper.parse_ref("note-#{uuid}")).to eq(uuid)
  end

  it 'rejects a slug with no trailing UUID' do
    expect { Temper.parse_ref('just-a-slug') }.to raise_error(ArgumentError, /not a ref/)
  end

  it 'rejects an empty string' do
    expect { Temper.parse_ref('') }.to raise_error(ArgumentError)
  end

  it 'rejects a truncated UUID' do
    expect { Temper.parse_ref('slug-019f4912-3f20-7fd3-814f') }.to raise_error(ArgumentError)
  end

  it 'rejects a non-String' do
    expect { Temper.parse_ref(nil) }.to raise_error(ArgumentError, /String/)
  end

  it 'rejects a UUID with a trailing suffix' do
    expect { Temper.parse_ref("#{uuid}-extra") }.to raise_error(ArgumentError)
  end
end
