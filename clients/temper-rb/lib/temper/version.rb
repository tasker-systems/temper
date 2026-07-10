# frozen_string_literal: true

module Temper
  # The gem's own SemVer. Independent of the API contract by design (D16):
  # a gem version and an API version answer different questions.
  #
  # This file is loaded by temper-rb.gemspec via require_relative, so it must
  # stand alone: no reference to Temper::Generated, which is not loaded then.
  # CONTRACT_VERSION is defined in lib/temper.rb, after the generated core.
  VERSION = '0.1.0'
end
