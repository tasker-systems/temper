# frozen_string_literal: true

# `Temper` must exist before any generated file is required: every generated
# file opens with the compact form `module Temper::Generated`, which raises
# NameError if `Temper` is not already defined.
module Temper
  # Process-wide settings. Credentials are NOT here -- they are per-call (D12).
  class Configuration
    attr_accessor :base_url, :device_id
  end

  class << self
    def config
      @config ||= Configuration.new
    end

    def configure
      yield(config)
      config
    end
  end
end

# `module Temper` must exist before anything generated is required: every
# generated file opens with the compact form `module Temper::Generated`.
require 'temper/generated'
require 'temper/version'
require 'temper/errors'
require 'temper/credentials'
require 'temper/connection'
require 'temper/act'
require 'temper/refs'

# The contract this gem was generated against. `rake generate` passes
# openapi.json's info.version to the generator as `gemVersion`, so the generated
# tree already carries it -- we alias it rather than reasserting it, and callers
# never reach into Temper::Generated for it.
Temper::CONTRACT_VERSION = Temper::Generated::VERSION
