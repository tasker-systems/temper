# frozen_string_literal: true

require_relative 'lib/temper/version'

Gem::Specification.new do |spec|
  spec.name          = 'temper-rb'
  spec.version       = Temper::VERSION
  spec.authors       = ['Pete Taylor']
  spec.email         = ['pete.jc.taylor@hey.com']

  spec.summary       = 'Ruby SDK for the Temper knowledge-base API'
  spec.description   = <<~DESC
    A pure-Ruby client for the Temper cloud API: resources, contexts, ingest,
    search, graph, and incremental cognitive-map authoring. No native extension.
  DESC

  spec.homepage      = 'https://github.com/tasker-systems/temper'
  spec.license       = 'MIT'
  spec.required_ruby_version = '>= 3.1'

  spec.metadata['homepage_uri']          = spec.homepage
  spec.metadata['source_code_uri']       = 'https://github.com/tasker-systems/temper/tree/main/clients/temper-rb'
  spec.metadata['bug_tracker_uri']       = 'https://github.com/tasker-systems/temper/issues'
  spec.metadata['allowed_push_host']     = 'https://rubygems.org'
  spec.metadata['rubygems_mfa_required'] = 'true'

  spec.files = Dir['lib/**/*.rb', 'README.md', 'LICENSE'].select { |f| File.file?(f) }
  spec.require_paths = ['lib']

  # Faraday's 2.x floor is forced by faraday-net_http_persistent 2.x (D11).
  spec.add_dependency 'faraday', '>= 2.5', '< 3.0'
  # faraday-multipart and marcel are required unconditionally by the GENERATED
  # api_client.rb. Both are pure Ruby.
  spec.add_dependency 'faraday-multipart', '~> 1.0'
  spec.add_dependency 'faraday-net_http_persistent', '~> 2.0'
  spec.add_dependency 'marcel', '~> 1.0'
  # Transitive via net-http-persistent, but pinned explicitly because we DEPEND on
  # its behaviour: connection_pool >= 2.4 defaults `auto_reload_after_fork: true`
  # and drops pooled sockets from a Process._fork hook. That, not
  # Temper.reset_connection!, is what stops a forked Puma/Sidekiq worker from
  # riding its parent's socket. Proven in spec/temper/fork_safety_spec.rb.
  spec.add_dependency 'connection_pool', '>= 2.4', '< 4.0'
end
