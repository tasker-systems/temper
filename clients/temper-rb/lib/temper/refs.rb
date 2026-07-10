# frozen_string_literal: true

module Temper
  # A hyphenated UUID. Deliberately narrower than Rust's `Uuid::parse_str`, which
  # also accepts the simple (unhyphenated), braced, and URN forms: this gem only
  # ever hands the string straight back to the caller to put in a URL, so widening
  # the accept set would mean emitting a non-canonical id. Narrower is safe --
  # it rejects only inputs the server would have accepted.
  UUID_PATTERN = /\A\h{8}-\h{4}-\h{4}-\h{4}-\h{12}\z/
  private_constant :UUID_PATTERN

  # Resolve a ref to its UUID. Accepts a bare UUID or the decorated
  # `sluggify(title)-<uuid>` form; resolution is trailing-UUID-only, so a stale
  # slug half is harmless. No fuzzy matching -- unparseable input is an error,
  # never a guess.
  #
  # A pure port of temper_workflow::operations::parse_ref. There is no by-slug
  # lookup, so this never touches the network. The gem does NOT port `sluggify`:
  # the server derives the slug from the title.
  def self.parse_ref(ref)
    raise ArgumentError, 'ref must be a String' unless ref.is_a?(String)

    ref = ref.strip
    return ref if ref.match?(UUID_PATTERN)

    # A UUID contains four internal hyphens, so it is the last five
    # hyphen-delimited groups. Walk from the right.
    parts = ref.split('-')
    if parts.length >= 5
      tail = parts.last(5).join('-')
      return tail if tail.match?(UUID_PATTERN)
    end

    raise ArgumentError, "not a ref (expected a UUID or `slug-<uuid>`): #{ref.inspect}"
  end
end
