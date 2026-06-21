-- Portable public.uuid_generate_v7() for the PG17 flip container.
--
-- The flip container is a bare pgvector PG17 image: it has neither Neon's
-- `pg_uuidv7` extension nor PG18's native `uuidv7()`, and prod's `public` dump
-- supplies `uuid_generate_v7()` via the pg_uuidv7 EXTENSION (whose CREATE EXTENSION
-- cannot restore here). Synthesis calls uuid_generate_v7() for internal chunk/event/
-- revision ids (external resource/profile/context ids are preserved verbatim), so
-- the container needs a working generator. These internal ids only need to be valid,
-- unique, time-sortable v7 UUIDs — the exact bytes are immaterial.
--
-- Build a v7 UUID from current epoch millis (48-bit timestamp) + random bits, with
-- the version (7) and variant (10) nibbles set per RFC 9562.
CREATE OR REPLACE FUNCTION public.uuid_generate_v7() RETURNS uuid
LANGUAGE sql VOLATILE PARALLEL SAFE AS $$
  SELECT encode(
    set_bit(
      set_bit(
        overlay(
          uuid_send(gen_random_uuid())
          PLACING substring(int8send((extract(epoch FROM clock_timestamp()) * 1000)::bigint) FROM 3)
          FROM 1 FOR 6
        ),
        52, 0
      ),
      53, 1
    ),
    'hex'
  )::uuid;
$$;
