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

require 'temper/version'
