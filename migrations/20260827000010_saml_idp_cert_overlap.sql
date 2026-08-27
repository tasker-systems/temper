-- An IdP signing-key rollover has an overlap window.
--
-- `kb_saml_idp` carries two certificate slots, so the set of acceptable assertion signers can hold
-- the outgoing and the incoming certificate at once. That overlap is what makes each step of a
-- rotation independently reversible: add the incoming cert, confirm logins, let the IdP cut over,
-- then drop the outgoing one. node-saml's `idpCert` accepts `string[]`, so both are offered to
-- signature verification.
--
-- PURELY ADDITIVE: one nullable column and two NOT VALID constraints. NULL means "no rollover in
-- progress", which is every existing row and the steady state. No Rust `sqlx::query!` macro reads
-- this table -- the SAML authorization server lives entirely in the temper-cloud TypeScript layer
-- -- so the committed `.sqlx/` cache is unaffected, same as the migration that created the table.
--
-- The one-active-IdP invariant is UNCHANGED and deliberately so. `loadActiveIdp`'s
-- `WHERE is_active = true LIMIT 1` selects one logical IdP; this column widens that one IdP's
-- signer set by exactly one named certificate. Relaxing the predicate instead would make a second
-- IdP's cert a valid signer, which is a different and much larger change.
ALTER TABLE kb_saml_idp ADD COLUMN idp_cert_secondary TEXT;

COMMENT ON COLUMN kb_saml_idp.idp_cert_secondary IS
    'Second acceptable signing certificate for this IdP during a signing-key rollover. NULL outside '
    'a rollover. Order carries no meaning to verification -- an assertion signed by either this or '
    'idp_cert is accepted -- so a cutover is one statement: '
    'SET idp_cert = idp_cert_secondary, idp_cert_secondary = NULL.';

-- A slot that is present but blank is the shape that reads as configured and is not.
--
-- Two failures it forecloses, both reachable from an ordinary scripted rotation whose cert variable
-- is unset. `idp_cert_secondary = ''` would survive the confirm step of a rotation -- the primary is
-- still signing, so logins pass -- and then the cutover statement moves the blank into `idp_cert`,
-- where NOT NULL does not fire and the working certificate is gone with no copy in the row. And a
-- blank in either slot makes every assertion fail signature verification, which is indistinguishable
-- from a genuinely bad assertion and sends an operator to debug the IdP rather than the row.
--
-- NOT VALID: new and updated rows are checked, existing rows are not scanned. That keeps this
-- migration additive -- it cannot fail on apply against data already in the table -- while still
-- refusing the write that would create the state.
ALTER TABLE kb_saml_idp
    ADD CONSTRAINT kb_saml_idp_certs_non_blank
    CHECK (
        btrim(idp_cert) <> ''
        AND (idp_cert_secondary IS NULL OR btrim(idp_cert_secondary) <> '')
    ) NOT VALID;

SELECT declare_migration(
    20260827000010,
    'additive',
    'One nullable TEXT column on kb_saml_idp, a COMMENT, and one NOT VALID CHECK. Nothing pre-existing is altered, renamed or dropped, and the CHECK is NOT VALID so no existing row is scanned and the migration cannot fail on apply against data already in the table. Both directions of the binary/schema contract hold. An older binary keeps working: its loadActiveIdp names its columns explicitly and does not mention idp_cert_secondary, so it reads the same row and offers node-saml the same single certificate; the column is NULL on every existing row and on every INSERT that omits it, including the one temper-cli''s admin_saml renders, so it never meets a value it cannot interpret. A NEWER binary also keeps working against the OLD schema, which matters because migrations here are a deploy step rather than a startup step and the self-host playbooks deploy before migrating: loadActiveIdp reads the new column as to_jsonb(t) ->> ''idp_cert_secondary'' rather than naming it, which yields NULL where the column is absent instead of raising 42703. No Rust sqlx::query! macro reads this table; the SAML authorization server lives entirely in the temper-cloud TypeScript layer, so the committed .sqlx cache is unaffected, as it was for the migration that created the table. Note that this also means the .sqlx drift check cannot see this table''s readers at all -- the new-binary-against-old-schema direction above is held by loadActiveIdp''s to_jsonb read and by tests, not by that check.'
);
