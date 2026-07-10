# frozen_string_literal: true

module Temper
  class Contexts
    def initialize(client)
      @client = client
    end

    # POST /api/contexts -> 201 ContextRow.
    #
    # ContextCreateRequest's fields are `name` and `owner`; only `name` is
    # required, and there is no `slug` on the wire -- the server derives it.
    def create(name:, owner: nil)
      attrs = { name: name }
      attrs[:owner] = owner_ref(owner) unless owner.nil?
      body = Generated::ContextCreateRequest.new(attrs)
      @client.call { |api| Generated::ContextsApi.new(api).create_context(body) }
    end

    def list(**filters)
      @client.call(idempotent: true) { |api| Generated::ContextsApi.new(api).list_contexts(filters) }
    end

    def show(ref)
      id = Temper.parse_ref(ref)
      @client.call(idempotent: true) { |api| Generated::ContextsApi.new(api).get_context(id) }
    end

    private

    # ContextOwnerRef is an externally-tagged serde enum mixing a unit variant
    # with newtype variants, so it has no discriminator and the generated oneOf
    # wrapper is best-effort by its own admission ("we do not attempt to check
    # whether exactly one item matches"). We build the wire shape directly and
    # let the generated model carry it: to_hash passes non-model values verbatim.
    #
    # Unencodable input is an error, never a guess.
    def owner_ref(owner)
      case owner
      when :me, 'me' then 'Me'
      when Hash then hash_owner_ref(owner)
      else
        raise ArgumentError, "owner must be :me, {team:}, or {profile:}, got #{owner.inspect}"
      end
    end

    def hash_owner_ref(owner)
      return { 'Team' => owner.fetch(:team) } if owner.key?(:team)
      return { 'Profile' => owner.fetch(:profile) } if owner.key?(:profile)

      raise ArgumentError, "owner Hash must carry :team or :profile, got #{owner.keys.inspect}"
    end
  end
end
