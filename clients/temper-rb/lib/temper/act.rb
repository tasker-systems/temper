# frozen_string_literal: true

module Temper
  # Act context for a write. Optional on every call.
  #
  # The constructor invariant mirrors ActInput::into_act_context: Rust's
  # AgentAuthorship.confidence is non-Option, so authorship without confidence is
  # a 400. Rejecting it here is the parse-don't-validate answer -- an invalid Act
  # cannot be constructed, so no call site can send one.
  #
  # `correlation` and `invocation` are exempt: correlation is provenance, never
  # authorship, and an act with no supplied correlation self-roots to its own
  # event id. Nothing gates on it, so the gem may always omit it.
  class Act
    AUTHORSHIP_FIELDS = %i[reasoning rationale persona model].freeze

    def initialize(confidence: nil, reasoning: nil, rationale: nil, persona: nil, model: nil,
                   correlation: nil, invocation: nil)
      authorship = { reasoning: reasoning, rationale: rationale, persona: persona, model: model }
      require_confidence!(confidence, authorship)

      @fields = authorship.merge(
        confidence: confidence&.to_s,
        correlation_id: correlation,
        invocation_id: invocation
      ).compact.freeze
    end

    # Symbol-keyed, nils omitted: the server distinguishes an absent key from null.
    # These are the seven ActInput wire keys, which flatten into ~30 write bodies
    # and onto the query string of DELETE /api/resources/{id}.
    def to_h
      @fields.dup
    end

    private

    def require_confidence!(confidence, authorship)
      return unless confidence.nil?

      supplied = authorship.compact.keys
      return if supplied.empty?

      raise ArgumentError, "Temper::Act requires `confidence:` when supplying #{supplied.join(', ')}"
    end
  end
end
