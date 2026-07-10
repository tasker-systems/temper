# frozen_string_literal: true

require 'temper'
require 'webmock/rspec'

# The repo-root contract. Anchored here rather than in each spec, so a spec that
# moves a directory deeper does not silently point at the wrong file.
CONTRACT_PATH = File.expand_path('../../../openapi.json', __dir__)

RSpec.configure do |config|
  config.expect_with(:rspec) { |c| c.syntax = :expect }
  config.disable_monkey_patching!
  config.order = :random
  Kernel.srand config.seed
end
