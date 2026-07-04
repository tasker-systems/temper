-- T7c Task 9 (storage half) — 'remote' provenance storage + resolution + read surfacing.
--
-- A remote source is an external URL. We mint a UUID per distinct URL in `kb_remote_sources` so the
-- uniform "`source_id` is always a UUID" invariant — leaned on by both projectors, the read fn, and
-- `kb_block_provenance`'s UNIQUE key — holds unchanged; the migration stays purely additive (new table
-- + helpers + a CREATE OR REPLACE of the projector INSERT helper + a DROP/CREATE of the read fn to add
-- a surfaced-URL column). The enum value 'remote' was committed in …006 (separate transaction).

-- ── kb_remote_sources: one row per distinct external URL ──────────────────────
-- `uri` = the URL as supplied (display / audit); `uri_normalized` = the canonical dedup + find-by key.
-- Normalization is a pure, total, network-free string function, so re-projecting the same event yields
-- the identical key (replay-stable) and normalization-equivalent URLs collapse to one row.
CREATE TABLE kb_remote_sources (
    id             uuid PRIMARY KEY DEFAULT uuid_generate_v7(),
    uri            text NOT NULL,
    uri_normalized text NOT NULL UNIQUE,
    first_seen     timestamptz NOT NULL DEFAULT now()
);

-- ── normalize_remote_uri: conservative, total, pure ───────────────────────────
-- Trim; lowercase the scheme + authority ONLY (never the path/query/fragment — those can be
-- case-significant); strip the default port (:80 for http, :443 for https); drop a lone trailing slash
-- on an empty path. Deliberately does NOT touch query strings or fragments (semantically load-bearing).
-- Non-URL input is returned trimmed, unchanged (total — never raises), so it is safe on any string.
CREATE FUNCTION normalize_remote_uri(p_uri text) RETURNS text
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v        text := btrim(p_uri);
    v_parts  text[];
    v_auth   text;
    v_rest   text;
    v_scheme text;
BEGIN
    -- (scheme://authority)(path/query/fragment). Authority stops at the first '/', '?' or '#'.
    v_parts := regexp_match(v, '^([a-zA-Z][a-zA-Z0-9+.-]*://[^/?#]*)(.*)$');
    IF v_parts IS NULL THEN
        RETURN v;  -- not an absolute URL; total fallthrough
    END IF;
    v_auth   := lower(v_parts[1]);
    v_rest   := v_parts[2];
    v_scheme := split_part(v_auth, '://', 1);
    IF v_scheme = 'http' THEN
        v_auth := regexp_replace(v_auth, ':80$', '');
    ELSIF v_scheme = 'https' THEN
        v_auth := regexp_replace(v_auth, ':443$', '');
    END IF;
    IF v_rest = '/' THEN
        v_rest := '';  -- empty path: "https://h/" ⇒ "https://h"
    END IF;
    RETURN v_auth || v_rest;
END;
$$;

-- ── _upsert_remote_source: resolve a URL to its kb_remote_sources id ───────────
-- Idempotent on `uri_normalized`. DO UPDATE (not DO NOTHING) so the existing id is RETURNED on
-- conflict; the SET is a no-op self-assignment that preserves the first-seen raw `uri`.
CREATE FUNCTION _upsert_remote_source(p_uri text) RETURNS uuid
LANGUAGE plpgsql AS $$
DECLARE v_id uuid;
BEGIN
    INSERT INTO kb_remote_sources (uri, uri_normalized)
    VALUES (p_uri, normalize_remote_uri(p_uri))
    ON CONFLICT (uri_normalized) DO UPDATE SET uri = kb_remote_sources.uri
    RETURNING id INTO v_id;
    RETURN v_id;
END;
$$;

-- ── _insert_block_provenance: teach the source-INSERT helper the 'remote' branch ──
-- Body copied verbatim from …003 with ONE change: a 'remote' source's value is a URL (not a UUID), so
-- resolve it to a kb_remote_sources id before the INSERT; resource/event values remain the id itself.
CREATE OR REPLACE FUNCTION _insert_block_provenance(p_block uuid, p_event uuid, p_incorporated jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_inc jsonb; v_kind text; v_val text; v_source_id uuid;
BEGIN
    IF p_incorporated IS NULL OR jsonb_typeof(p_incorporated) <> 'array' THEN
        RETURN;
    END IF;
    FOR v_inc IN SELECT jsonb_array_elements(p_incorporated) LOOP
        v_kind := v_inc #>> '{source,kind}';
        v_val  := v_inc #>> '{source,value}';
        IF v_kind = 'remote' THEN
            v_source_id := _upsert_remote_source(v_val);   -- URL → minted/looked-up kb_remote_sources id
        ELSE
            v_source_id := v_val::uuid;                    -- resource/event: the value IS the id
        END IF;
        INSERT INTO kb_block_provenance
            (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq)
        VALUES (p_block, v_kind::provenance_source_kind, v_source_id, p_event, (v_inc ->> 'seq')::int)
        ON CONFLICT (block_id, source_kind, source_id, contributed_by_event_id) DO NOTHING;
    END LOOP;
END;
$$;

-- ── resource_block_provenance: surface the raw URL for remote rows ─────────────
-- DROP + CREATE (not CREATE OR REPLACE) because the RETURNS TABLE gains a column (`source_uri`), which
-- CREATE OR REPLACE cannot do. LEFT JOIN kb_remote_sources so a 'remote' row surfaces the human URL and
-- resource/event rows return NULL. Same access gate + ordering as …003.
DROP FUNCTION IF EXISTS resource_block_provenance(uuid, text, uuid);
CREATE FUNCTION resource_block_provenance(
    p_resource uuid, p_principal_kind text, p_principal_id uuid
) RETURNS TABLE(block_id uuid, block_seq int, source_kind text, source_id uuid, source_uri text,
                accretion_seq int, contributed_by_event_id uuid, created timestamptz)
LANGUAGE sql STABLE AS $$
    SELECT b.id, b.seq, p.source_kind::text, p.source_id, r.uri, p.accretion_seq,
           p.contributed_by_event_id, p.created
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    LEFT JOIN kb_remote_sources r ON p.source_kind = 'remote' AND r.id = p.source_id
    WHERE b.resource_id = p_resource AND NOT b.is_folded
      AND p_resource IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ORDER BY b.seq, p.accretion_seq;
$$;
