# frozen_string_literal: true

require 'ipaddr'
require 'uri'

module Temper
  # What this gem is willing to put a secret on, checked once, at the seam.
  #
  # Two URLs arrive from a caller and then carry credentials on every use: the
  # configured base URL -- every request puts the bearer token on it -- and the
  # token URL, where every mint puts the client_secret on the wire. For both,
  # the scheme is not cosmetic: plaintext http off the loopback interface puts
  # the credential in the clear, readable by anything on the path.
  #
  # Checked at CONSTRUCTION, not at first use -- the same discipline the sibling
  # clients state (temper-py's temper/_validate.py, temper-client's endpoint.rs,
  # temper-ts's validate.ts): a URL validated when a request is built would
  # surface its error several layers and possibly several minutes from the
  # configuration that caused it.
  module Validate
    module_function

    # Hostnames that are the local machine by definition. `.localhost` is
    # reserved for exactly this by RFC 6761 6.3, and Docker/CI setups do use
    # `foo.localhost`.
    LOOPBACK_NAMES = ['localhost'].freeze

    # An absolute http(s) origin this gem is willing to put a secret on.
    # Returns the parsed URI.
    #
    # `allow_insecure_http` is the deliberate opt-out for the case this check
    # cannot see -- a private network where TLS terminates elsewhere. It is a
    # keyword a caller has to write, which is the whole point: it must not be a
    # typo away.
    def require_endpoint(value, name:, allow_insecure_http: false)
      raise ArgumentError, "#{name} must be a non-empty String" unless value.is_a?(String) && !value.empty?

      # URI.parse refuses most malformed input; ASCII whitespace and control
      # characters are refused up front (with the parser catching anything
      # non-ASCII) rather than normalized: a URL with an embedded tab or
      # newline is not a URL the caller wrote, and silently cleaning it hides
      # the mistake.
      return bad_chars(value, name) if value.match?(/[\s[:cntrl:]]/)

      uri = parse(value, name)

      # URI.parse hands back URI::FTP, URI::MailTo and friends just as happily;
      # only the two schemes that can protect a credential are accepted, and
      # only with a host.
      unless (uri.is_a?(URI::HTTP) || uri.is_a?(URI::HTTPS)) && !uri.host.to_s.empty?
        raise ArgumentError, "#{name} must be an absolute http(s) URL, got #{value.inspect}"
      end

      # `uri.user` is nil for `host:port`, so this catches only a real userinfo
      # section. Refused rather than dropped: a caller who wrote credentials
      # into the URL meant them to authenticate something, and quietly
      # discarding them would produce a 401 whose cause is invisible.
      if uri.user || uri.password
        raise ArgumentError,
              "#{name} must not carry userinfo (user:password@); pass credentials to ClientCredentials instead"
      end

      if uri.query || uri.fragment
        raise ArgumentError,
              "#{name} must be an origin (optionally with a path prefix), not a URL with a query or fragment"
      end

      if uri.scheme == 'http' && !(allow_insecure_http || loopback?(uri.host))
        raise ArgumentError,
              "#{name} is plaintext http to a non-loopback host, which would put the bearer token " \
              'and client_secret on the wire in the clear; use https, or pass ' \
              'allow_insecure_http: true to accept that deliberately'
      end

      uri
    end

    # Whether `hostname` names this machine, by literal address or by reserved
    # name. URI#host keeps the brackets on an IPv6 literal, so they are stripped
    # before IPAddr sees the address -- it covers the whole 127.0.0.0/8 block,
    # not just 127.0.0.1.
    def loopback?(hostname)
      host = hostname.to_s.delete_prefix('[').delete_suffix(']').downcase.delete_suffix('.')
      begin
        return IPAddr.new(host).loopback?
      rescue IPAddr::InvalidAddressError
        nil
      end
      LOOPBACK_NAMES.include?(host) || host.end_with?('.localhost')
    end

    def parse(value, name)
      uri = URI.parse(value)
      # Ruby's URI accepts a port outside 0..65535 without complaint; the
      # address is not usable, so it meets the same message as a parse failure.
      unless uri.port.to_i.between?(0, 65_535)
        raise ArgumentError, "#{name} is not a parseable URL: #{value.inspect}"
      end
      uri
    rescue URI::InvalidURIError, URI::InvalidComponentError, URI::BadURIError
      raise ArgumentError, "#{name} is not a parseable URL: #{value.inspect}"
    end

    def bad_chars(value, name)
      raise ArgumentError, "#{name} must not contain whitespace or control characters"
    end
  end
end
