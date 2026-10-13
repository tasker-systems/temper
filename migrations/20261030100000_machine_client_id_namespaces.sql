-- A machine is never registered under a client id temper mints.
--
-- The machine gate (profile_service::resolve_machine_from_claims) finds a machine by client id
-- alone, whoever issued the token. temper's Authorization Server issues client_credentials tokens to
-- two kinds of client: temper-issued machine credentials (kb_machine_clients, issuer 'temper', ids
-- beginning tmpr_) and dynamically registered clients (kb_oauth_dcr_clients, ids beginning conn_).
-- A kb_machine_clients row from any other issuer under one of those ids would receive every token
-- issued to it: a DCR client's tokens would act with the registered machine's reach.
--
-- machine_registration_service refuses such an id at provision and rebind. These two constraints
-- make the separation hold in the schema too, whatever writes the row:
--
--   * kb_oauth_dcr_clients.client_id begins conn_, so a change to the id format in register.ts
--     fails at insert instead of silently leaving the reserved namespace.
--   * a kb_machine_clients row not issued by temper keeps out of conn_ and tmpr_.
--
-- Rows that already break either rule would stop this migration, by design: they are the capture
-- this exists to prevent, and must be looked at by hand before it can apply.

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM kb_oauth_dcr_clients WHERE client_id NOT LIKE 'conn\_%') THEN
        RAISE EXCEPTION 'kb_oauth_dcr_clients holds a client id outside conn_; inspect it before applying';
    END IF;
    IF EXISTS (
        SELECT 1 FROM kb_machine_clients
         WHERE issuer <> 'temper'
           AND (client_id LIKE 'conn\_%' OR client_id LIKE 'tmpr\_%')
    ) THEN
        RAISE EXCEPTION 'kb_machine_clients holds a non-temper client id in conn_ or tmpr_; inspect it before applying';
    END IF;
END
$$;

ALTER TABLE kb_oauth_dcr_clients
    ADD CONSTRAINT kb_oauth_dcr_clients_client_id_namespace
    CHECK (client_id LIKE 'conn\_%');

ALTER TABLE kb_machine_clients
    ADD CONSTRAINT kb_machine_clients_client_id_namespace
    CHECK (issuer = 'temper' OR (client_id NOT LIKE 'conn\_%' AND client_id NOT LIKE 'tmpr\_%'));

SELECT declare_migration(
    20261030100000,
    'additive',
    'Adds two CHECK constraints keeping machine and DCR client ids apart: kb_oauth_dcr_clients.client_id '
    'must begin conn_, and a kb_machine_clients row whose issuer is not temper may not begin conn_ or '
    'tmpr_. Refuses to apply if an existing row breaks either. No column, function or grant changes. '
    'A binary without this migration writes only rows that satisfy both: register.ts mints conn_ ids, '
    'issue mints tmpr_ ids with issuer temper, and provision and rebind write caller ids that the newer '
    'binary refuses in the reserved namespaces before the insert.'
);
