# frozen_string_literal: true

# A doc that drifts from the code is worse than no doc. These pin the claims a
# user ACTS on and cannot verify from the contract -- the ones that, when stale,
# send someone down a path that no longer exists.
#
# Two files are gated, because the docs are two files:
#
#   * README.md                -- ships in the gem (`spec.files`), so it is what a
#                                 Rubyist sees on rubygems.org. The quickstart.
#   * docs/guides/temper-rb.md -- the long-form how-to, in the repo's guides area
#                                 alongside every other integrator doc. The README
#                                 links out to it, and this gate holds that link.
#
# Reaching out of the gem dir is the same move credentials_spec makes for
# tests/contracts/m2m-token-request.json: the suite runs from the monorepo.
#
# The NEGATIVE assertions are the load-bearing ones. This gate was previously
# green against a README whose "Going live" said step 1 was "provision an Auth0
# M2M application" and that "there is no self-serve path -- an operator runs
# this" -- both false since the temper-issued mint path (Phase B1) and team-owner
# registration (Phase B2). A gate that only checks for presence will happily keep
# a lie green, which is exactly what this one did.
RSpec.describe 'README' do
  let(:readme) { File.read(File.expand_path('../README.md', __dir__)) }
  let(:guide) { File.read(File.expand_path('../../../docs/guides/temper-rb.md', __dir__)) }
  let(:docs) { [readme, guide] }

  # The README ships; the guide does not. So the README must stand on its own AND
  # get the reader to the guide -- with an ABSOLUTE url, because a repo-relative
  # link is dead on rubygems.org.
  describe 'the README, which is what rubygems.org renders' do
    it 'links to the guide with a url that survives leaving the repo' do
      expect(readme).to include('https://github.com/tasker-systems/temper/blob/main/docs/guides/temper-rb.md')
    end

    it 'names both credential classes' do
      expect(readme).to include('Temper::Credentials::BearerToken')
      expect(readme).to include('Temper::Credentials::ClientCredentials')
    end

    it 'documents the four TEMPER_M2M_* variables by their real names' do
      %w[TEMPER_M2M_TOKEN_URL TEMPER_M2M_CLIENT_ID TEMPER_M2M_CLIENT_SECRET TEMPER_M2M_AUDIENCE]
        .each { |var| expect(readme).to include(var) }
    end

    it 'documents the fork-safety hooks' do
      expect(readme).to include('Temper.reset_connection!')
      expect(readme).to include('on_worker_boot')
      expect(readme).to include('Sidekiq.configure_server')
    end

    it 'names the boot-time assertion it tells you to make' do
      expect(readme).to include('whoami')
    end

    it 'explains why bulk reconcile is absent' do
      expect(readme).to include('reconcile')
      expect(readme).to include('chunks_packed')
    end
  end

  # Temper is fronted by TWO issuers. A doc that knows about only one sends a
  # temper-issued caller (a `tmpr_` client id, from `temper admin machine issue`)
  # to an Auth0 tenant that has never heard of it.
  describe 'both mint paths, in both docs' do
    it 'names both minting commands' do
      expect(docs).to all(include('temper admin machine provision'))
      expect(docs).to all(include('temper admin machine issue'))
    end

    it 'says audience is required for Auth0 and omitted for a temper-issued credential' do
      expect(docs).to all(include('AUTH_AUDIENCE'))
      expect(docs).to all(match(/omit it/i))
    end

    # `audience` became optional on ClientCredentials, and the mandatory `ENV.fetch`
    # in the old README's snippet is the sharp end of the stale claim: it RAISES on
    # a temper-issued deploy, which has no audience to set. A doc that still shows
    # it is not merely out of date -- it is a KeyError in someone's initializer.
    it 'never fetches TEMPER_M2M_AUDIENCE as a mandatory variable' do
      docs.each { |doc| expect(doc).not_to include("ENV.fetch('TEMPER_M2M_AUDIENCE')") }
    end

    it 'never presents audience as unconditionally required' do
      docs.each { |doc| expect(doc).not_to match(/audience`?\s*must equal the API's configured/i) }
    end

    it 'names the tmpr_ prefix a temper-issued client id actually carries' do
      expect(guide).to include('tmpr_')
    end
  end

  # Registration is the first wall; reach is the second. Both docs must say so,
  # and must not claim the wall is taller than it is.
  describe 'going live' do
    it 'carries a Going live section naming registration and both reach flags' do
      expect(docs).to all(match(/##\s*Going live/i))
      expect(docs).to all(include('cogmap write grant'))
      expect(docs).to all(include('team membership'))
    end

    # Phase B2: minting is `is_system_admin` OR owner of the owning team. "An
    # operator runs this" was true in Phase A and is now too strong -- it tells a
    # team owner to go file a ticket they do not need.
    it 'never claims a machine principal has no self-serve path' do
      docs.each do |doc|
        expect(doc).not_to match(/no self-serve path/i)
        expect(doc).not_to match(/an operator runs this/i)
      end
    end

    it 'says a team owner may register their own team machine' do
      expect(docs).to all(match(/team owner/i))
    end
  end

  # rotate-secret and rebind are different operations with different blast radii.
  # The README used to document only rebind, which is the WRONG one to reach for
  # when rolling a temper-issued secret -- and it is admin-only, so the advice
  # also sent a team owner to an admin they did not need.
  describe 'rotation' do
    it 'documents rotate-secret, not only rebind' do
      expect(docs).to all(include('rotate-secret'))
      expect(docs).to all(include('rebind'))
    end

    it 'distinguishes them: a new secret versus a new client_id' do
      expect(guide).to match(/grace window/i)
      expect(guide).to match(/system.admin only/i)
      expect(readme).to include('new `client_id`')
    end
  end

  # The guide is the one with room to explain, so it carries the claims the README
  # only gestures at. Each of these is backed by a spec elsewhere in this suite.
  describe 'the guide' do
    it 'grounds the token lifecycle -- skew, mutex, re-mint on 401' do
      expect(guide).to include('60s')
      expect(guide).to match(/mutex/i)
      expect(guide).to match(/re-mint/i)
    end

    it 'says a BearerToken cannot refresh' do
      expect(guide).to match(/cannot refresh/i)
    end

    it 'carries the retry rule that keeps a write from being re-submitted' do
      expect(guide).to match(/never auto-retried/i)
      expect(guide).to include('SystemAccessRequired')
    end

    it 'names the surface header the gem stamps and the wire key Act renames to' do
      expect(guide).to include('X-Temper-Surface: sdk')
      expect(guide).to include('correlation_id')
    end

    it 'points at the operator guide rather than restating it' do
      expect(guide).to include('machine-credentials.md')
    end
  end
end
