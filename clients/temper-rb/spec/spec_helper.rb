# frozen_string_literal: true

require 'temper'
require 'webmock/rspec'

# The repo-root contract. Anchored here rather than in each spec, so a spec that
# moves a directory deeper does not silently point at the wrong file.
CONTRACT_PATH = File.expand_path('../../../openapi.json', __dir__)

# Shared response fixtures.
#
# The generated models VALIDATE on deserialize: a non-nullable attribute that
# arrives nil raises ArgumentError from its setter. ResourceDetail is
# `allOf: [ResourceRow, {...}]`, and the generator flattens ResourceRow's ten
# required fields onto it -- so a stub returning `{}` fails inside the client,
# not in the assertion. Stub with a real row.
module Fixtures
  module_function

  def resource_row(id: '019f4912-3f20-7fd3-814f-13a5ddbe3cd7', **overrides)
    {
      id: id,
      origin_uri: '',
      title: 'A Resource',
      originator_profile_id: '019d4add-f49d-7c43-a87d-dda470e5dd9c',
      owner_profile_id: '019d4add-f49d-7c43-a87d-dda470e5dd9c',
      is_active: true,
      created: '2026-07-10T12:00:00Z',
      updated: '2026-07-10T12:00:00Z',
      doc_type_name: 'note',
      owner_handle: 'j-cole-taylor'
    }.merge(overrides)
  end

  def resource_row_json(...) = JSON.generate(resource_row(...))

  def context_row(id: '00000000-0000-0000-0003-000000000001', **overrides)
    {
      id: id,
      name: 'incidents',
      kb_owner_table: 'kb_profiles',
      kb_owner_id: '019d4add-f49d-7c43-a87d-dda470e5dd9c',
      created: '2026-07-10T12:00:00Z',
      updated: '2026-07-10T12:00:00Z',
      slug: 'incidents',
      owner_ref: '@j-cole-taylor'
    }.merge(overrides)
  end

  def context_row_json(...) = JSON.generate(context_row(...))
end

RSpec.configure do |config|
  config.expect_with(:rspec) { |c| c.syntax = :expect }
  config.disable_monkey_patching!
  config.order = :random
  Kernel.srand config.seed
end
