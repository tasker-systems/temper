# frozen_string_literal: true

require 'json'

RSpec.describe Temper::ErrorMapper do
  def api_error(status, body, headers = {})
    Temper::Generated::ApiError.new(code: status, response_headers: headers, response_body: body)
  end

  def envelope(code, message, details = nil)
    JSON.generate(error: { code: code, message: message, details: details })
  end

  it 'maps 409 to a permanent Conflict carrying the envelope' do
    err = described_class.call(api_error(409, envelope('CONFLICT', 'already exists')))
    expect(err).to be_a(Temper::Conflict)
    expect(err).to be_a(Temper::PermanentError)
    expect(err.status).to eq(409)
    expect(err.code).to eq('CONFLICT')
    expect(err.message).to eq('already exists')
  end

  it 'discriminates SystemAccessRequired off error.code, not the status' do
    err = described_class.call(api_error(403, envelope('SYSTEM_ACCESS_REQUIRED', 'grant needed')))
    expect(err).to be_a(Temper::SystemAccessRequired)
    expect(err).to be_a(Temper::Forbidden)
  end

  it 'maps a plain 403 to Forbidden, not SystemAccessRequired' do
    err = described_class.call(api_error(403, envelope('FORBIDDEN', 'nope')))
    expect(err).to be_a(Temper::Forbidden)
    expect(err).not_to be_a(Temper::SystemAccessRequired)
  end

  # The refusal is what makes the 403 actionable rather than merely final. The envelope arrives
  # with STRING keys and the generated oneOf dispatcher matches on symbols, so a raw hand-off
  # resolves to nil silently -- #refusal absorbs that, and these prove it stays absorbed.
  describe 'the typed refusal on SystemAccessRequired' do
    def refused(refusal)
      described_class.call(
        api_error(403, envelope('SYSTEM_ACCESS_REQUIRED', 'grant needed', { 'refusal' => refusal }))
      )
    end

    it 'resolves the string-keyed envelope into a named model' do
      err = refused({ 'kind' => 'revoked' })
      expect(err.refusal).to be_a(Temper::Generated::Revoked)
      expect(err.refusal_kind).to eq('revoked')
    end

    it 'separates never-granted from granted-and-lost' do
      expect(refused({ 'kind' => 'denied' }).refusal).to be_a(Temper::Generated::Denied)
      expect(refused({ 'kind' => 'revoked' }).refusal).to be_a(Temper::Generated::Revoked)
    end

    it 'carries the payload of a data-bearing refusal' do
      err = refused({ 'kind' => 'illegal_transition', 'act' => 'approve', 'from' => 'denied' })
      expect(err.refusal).to be_a(Temper::Generated::IllegalTransition)
      expect(err.refusal.act).to eq('approve')
      expect(err.refusal.from).to eq('denied')
    end

    # A server newer than the gem. Losing the name entirely would leave the operator with nothing.
    it 'still reports the kind of a refusal it cannot resolve' do
      err = refused({ 'kind' => 'something_new' })
      expect(err.refusal).to be_nil
      expect(err.refusal_kind).to eq('something_new')
    end

    it 'is nil, not an error, when the server sent no refusal at all' do
      err = described_class.call(api_error(403, envelope('SYSTEM_ACCESS_REQUIRED', 'grant needed')))
      expect(err.refusal).to be_nil
      expect(err.refusal_kind).to be_nil
    end
  end

  it 'surfaces error.details' do
    err = described_class.call(api_error(400, envelope('BAD_REQUEST', 'bad', { 'field' => 'title' })))
    expect(err).to be_a(Temper::BadRequest)
    expect(err.details).to eq({ 'field' => 'title' })
  end

  it 'maps 5xx to a transient ServerError' do
    err = described_class.call(api_error(503, 'upstream down'))
    expect(err).to be_a(Temper::ServerError)
    expect(err).to be_a(Temper::TransientError)
  end

  it 'maps 429 to RateLimited and surfaces Retry-After' do
    err = described_class.call(api_error(429, 'slow down', { 'Retry-After' => '30' }))
    expect(err).to be_a(Temper::RateLimited)
    expect(err.retry_after).to eq(30)
  end

  it 'tolerates a missing or unparseable Retry-After' do
    expect(described_class.call(api_error(429, 'slow')).retry_after).to be_nil
    expect(described_class.call(api_error(429, 'slow', { 'Retry-After' => 'Wed, 21 Oct' })).retry_after).to be_nil
  end

  # The generated ApiClient rescues Faraday::TimeoutError / ConnectionFailed and
  # re-raises them as ApiError with a NIL code. Without this branch every timeout
  # falls through to a bare Temper::Error and Sidekiq dead-letters it.
  it 'maps a nil-code ApiError (timeout / refused) to a transient ConnectionError' do
    err = described_class.call(Temper::Generated::ApiError.new('Connection timed out'))
    expect(err).to be_a(Temper::ConnectionError)
    expect(err).to be_a(Temper::TransientError)
    expect(err.message).to eq('Connection timed out')
  end

  it 'degrades to a raw-body detail when the response is not the envelope' do
    err = described_class.call(api_error(500, '<html>502 Bad Gateway</html>'))
    expect(err).to be_a(Temper::ServerError)
    expect(err.code).to be_nil
    expect(err.details).to eq('<html>502 Bad Gateway</html>')
  end

  it 'degrades when the body is valid JSON but not the envelope' do
    err = described_class.call(api_error(500, '{"oops":true}'))
    expect(err.code).to be_nil
    expect(err.details).to eq('{"oops":true}')
  end

  it 'maps 422 to BadRequest even though no operation declares it' do
    err = described_class.call(api_error(422, envelope('UNPROCESSABLE', 'bad shape')))
    expect(err).to be_a(Temper::BadRequest)
  end

  it 'maps 401 to Unauthorized and 404 to NotFound' do
    expect(described_class.call(api_error(401, '{}'))).to be_a(Temper::Unauthorized)
    expect(described_class.call(api_error(404, '{}'))).to be_a(Temper::NotFound)
  end

  it 'falls back to a bare Error for an unclassified status' do
    err = described_class.call(api_error(302, 'redirect'))
    expect(err.class).to eq(Temper::Error)
    expect(err).not_to be_a(Temper::TransientError)
    expect(err).not_to be_a(Temper::PermanentError)
  end

  it 'every Temper::Error carries status, code, message, details' do
    err = described_class.call(api_error(409, envelope('CONFLICT', 'dup', { 'id' => 'x' })))
    expect(err).to respond_to(:status, :code, :message, :details)
  end
end
