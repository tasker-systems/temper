-- Canonical _sqlx_migrations baseline — captured from a clean `cargo sqlx migrate run`.
-- Real sqlx checksums (sha384 of each migration file). Do not edit by hand.
-- Regenerate: clean public, `cargo sqlx migrate run --source migrations`, re-run the capture query.
INSERT INTO public._sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES
  (20260624000001, 'canonical schema', now(), true, E'\\xf849ba7692dd5adedc05898b304ba3a8d59895aac93283ab9e04f8ea019f3211a2d86a40ba26b6fc4c974e4a89876aae'::bytea, 42942709),
  (20260624000002, 'canonical functions', now(), true, E'\\x281062d74637ac0fa119d7ba22723eafc09131ae34fb211d3184a3c49eea10dc1ada654fe439fdf7ab516a42c1fa1214'::bytea, 5867541),
  (20260624000003, 'canonical seed', now(), true, E'\\x334837fdb87735790633a56b70c7011f2496c162db415301e17e5c7ed0be3023b864ec336f21b8154abaa743214984b1'::bytea, 2958042);
