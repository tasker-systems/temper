-- The Connect through-path probe's dynamic-client-registration store.
--
-- kb_oauth_dcr_clients holds clients registered through the AS-mode DCR door
-- (/oauth/clients, src/oauth/register.ts in packages/temper-cloud). LOAD-BEARING
-- DISJOINTNESS: this table is deliberately NOT kb_machine_clients and must never
-- be merged into it — a DCR'd client is not a machine principal: it holds no
-- grants, no team bindings, no profile row, and the Rust gate's
-- lookup-or-401 over kb_machine_clients refuses its tokens at every API surface.
-- That fail-closed refusal is the probe's measured containment boundary until the
-- Phase 1 build binds authority to these clients deliberately.
--
-- Probe-scoped by design: rows carry no revocation column because the preview
-- instance they serve is itself ephemeral; short token lifetimes
-- (AS_ACCESS_TTL_SECONDS) bound exposure until Phase 1 decides the real shape.

CREATE TABLE kb_oauth_dcr_clients (
    client_id                   TEXT PRIMARY KEY,
    client_secret_hash          TEXT NOT NULL,
    client_name                 TEXT,
    grant_types                 TEXT[] NOT NULL,
    redirect_uris               TEXT[] NOT NULL,
    token_endpoint_auth_method  TEXT NOT NULL,
    logo_uri                    TEXT,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_oauth_dcr_clients IS
  'OAuth clients registered through the AS-mode RFC 7591 DCR door (/oauth/clients). '
  'Probe-local by design and DISJOINT from kb_machine_clients: a DCR''d client is not a '
  'machine principal — no grants, no profile, refused at every Rust API gate '
  '(resolve_machine_from_claims lookup-or-401) until Phase 1 binds authority '
  'deliberately. Secret stored as sha256 hex (mint.ts hashToken), same as machine '
  'clients. Written only by packages/temper-cloud src/oauth/register.ts.';

SELECT declare_migration(
    20260904000010,
    'additive',
    'The Connect through-path probe''s DCR store (task '
    '01a06ca9-01e3-79d1-a876-c354cb0f023d): new table kb_oauth_dcr_clients for clients '
    'registered at the AS-mode /oauth/clients door (Vercel Connect DCR per the '
    '2026-08-29 trust-anchor decision). Deliberately disjoint from kb_machine_clients — '
    'DCR''d clients carry no grants and are refused at every Rust API gate until Phase 1 '
    'binds invocation-envelope authority. No existing column, constraint or function is '
    'altered; nothing reads the table except the registration and token endpoints.'
);
