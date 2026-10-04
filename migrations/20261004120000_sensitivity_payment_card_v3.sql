-- Q51: payment_card v3 requires a card issuer at a length that issuer uses.
--
-- Production's first sweep (2026-10-04) wrote 909 payment_card v2 findings. Measured on a scratch
-- branch of production, none held a match with a card issuer prefix: of 1,186 matches on the text
-- surfaces, 1,103 were 14-digit YYYYMMDDhhmmss migration stamps, and the rest were 14-digit runs
-- beginning 20, epoch milliseconds and 18-digit ids. Each passed Luhn, which one digit run in ten
-- does. Over every block, chunk and event payload, v2 makes 1,235 matches and v3 makes none.
--
-- The pattern and Q42's hex guard stand; only the validator changes. Production disabled v2 after
-- that sweep; v3 is enabled here, so the deploy that ships the fix also turns it on (0.6.0 item 3).
-- v2's findings do not wait for 3b's derived `superseded`: those v3 would not find again close now
-- as false positives (below), and 3b reads a finding's latest disposition before its supersession.

-- A prefix outside its lengths is not a card: Luhn alone passes one run of digits in ten. The
-- networks covered are those published with fixed prefixes and lengths; a card on a network not
-- listed (UATP, private-label) is not found.
CREATE FUNCTION sensitivity.card_valid(p_match text) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT CASE
               -- Visa
               WHEN d ~ '^4'                                             THEN length(d) IN (13, 16, 19)
               -- Mastercard
               WHEN d ~ '^(5[1-5]|222[1-9]|22[3-9]|2[3-6]|27[01]|2720)' THEN length(d) = 16
               -- Mir
               WHEN d ~ '^220[0-4]'                                      THEN length(d) BETWEEN 16 AND 19
               -- American Express
               WHEN d ~ '^3[47]'                                         THEN length(d) = 15
               -- JCB
               WHEN d ~ '^35(2[89]|[3-8])'                               THEN length(d) BETWEEN 16 AND 19
               -- Diners Club
               WHEN d ~ '^3(0[0-5]|095|[689])'                           THEN length(d) BETWEEN 14 AND 19
               -- Maestro, Dankort
               WHEN d ~ '^(5018|5019|5020|5038|5893|6304|6759|676[1-3])' THEN length(d) BETWEEN 13 AND 19
               -- Verve and Elo (506), RuPay (508, 60, 81, 82), Discover, InstaPayment, UnionPay.
               -- Wider than those networks on purpose (all of 60 and 62-65): the bound that matters
               -- is 16 digits, which no stamp, epoch or id measured in production reaches.
               WHEN d ~ '^(506|508|60|6[2-5]|8[12])'                     THEN length(d) BETWEEN 16 AND 19
               -- Troy
               WHEN d ~ '^9792'                                          THEN length(d) = 16
               ELSE false
           END
       AND sensitivity.luhn_valid(d)
      FROM (SELECT regexp_replace(p_match, '[^0-9]', '', 'g') AS d) m;
$$;

ALTER TABLE sensitivity.detectors DROP CONSTRAINT detectors_validator_check;
ALTER TABLE sensitivity.detectors ADD CONSTRAINT detectors_validator_check
    CHECK (validator IN ('ssn_valid', 'luhn_valid', 'aba_routing_valid', 'card_valid'));

-- Prior body: 20261002200000, plus the card_valid arm.
CREATE OR REPLACE FUNCTION sensitivity.detector_matches(p_detector text, p_version int, p_text text) RETURNS SETOF text
LANGUAGE sql STABLE STRICT AS $$
    SELECT m[1]
      FROM sensitivity.detector_versions d,
           LATERAL regexp_matches(p_text, '(' || d.pattern || ')', 'g') m
     WHERE d.detector_id = p_detector AND d.version = p_version
       AND p_text ~ d.prefilter
       AND m[1] <> ''
       AND CASE d.validator
               WHEN 'ssn_valid'         THEN sensitivity.ssn_valid(m[1])
               WHEN 'luhn_valid'        THEN sensitivity.luhn_valid(m[1])
               WHEN 'aba_routing_valid' THEN sensitivity.aba_routing_valid(m[1])
               WHEN 'card_valid'        THEN sensitivity.card_valid(m[1])
               ELSE true
           END
     LIMIT 100000;
$$;

UPDATE sensitivity.detectors
   SET version = version + 1,
       validator = 'card_valid',
       enabled = true,
       note = 'contiguous, or in card groupings with one separator, never inside a hex run; a card issuer at its length, behind Luhn'
 WHERE id = 'payment_card';

-- Closes each open payment_card finding of an earlier version that the current version would not
-- find again, as a false positive, and returns how many it closed. An operator need not resolve the
-- noise one finding at a time, and the record says it was noise.
--
-- Q27 carries a disposition across a bump by place and value, so a v3 finding where v2 was ruled
-- false would inherit the ruling. A finding therefore closes only when the current version finds
-- nothing in the units at its place: a jsonb place is read at its path. A finding stays open when
-- its place already closes (place_closure: the row changed, went or was erased; that says what
-- happened, and a disposition would not), when it carries any disposition, and when its unit cannot
-- be read, which includes a surface this function does not know.
CREATE FUNCTION sensitivity.close_cardless_card_findings() RETURNS int
LANGUAGE sql AS $$
    WITH current AS (
        SELECT version FROM sensitivity.detectors WHERE id = 'payment_card'
    ), candidates AS (
        SELECT f.id, f.surface, f.target_id, f.path, c.version
          FROM sensitivity.findings f, current c
         WHERE f.detector_id = 'payment_card' AND f.detector_version < c.version
           AND NOT EXISTS (SELECT 1 FROM sensitivity.dispositions x WHERE x.finding_id = f.id)
           AND sensitivity.place_closure(f.surface, f.target_id, f.content_hash) IS NULL
    ), units AS (
        SELECT k.id, u.unit,
               EXISTS (SELECT 1 FROM sensitivity.detector_matches('payment_card', k.version, u.unit)) AS card
          FROM candidates k
          CROSS JOIN LATERAL (
              SELECT s.unit FROM sensitivity.src_kb_block_content__content s
               WHERE k.surface = 'kb_block_content.content' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_chunk_content__content s
               WHERE k.surface = 'kb_chunk_content.content' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_chunks__header_path s
               WHERE k.surface = 'kb_chunks.header_path' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_resources__title s
               WHERE k.surface = 'kb_resources.title' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_resources__origin_uri s
               WHERE k.surface = 'kb_resources.origin_uri' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_properties__property_key s
               WHERE k.surface = 'kb_properties.property_key' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_edges__label s
               WHERE k.surface = 'kb_edges.label' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_citation_audits__reason s
               WHERE k.surface = 'kb_citation_audits.reason' AND s.target_id = k.target_id
              UNION ALL SELECT s.unit FROM sensitivity.src_kb_remote_sources__uri s
               WHERE k.surface = 'kb_remote_sources.uri' AND s.target_id = k.target_id
              UNION ALL SELECT j.unit FROM sensitivity.src_kb_events__payload s,
                                           sensitivity.jsonb_units(s.doc, s.roots) j
               WHERE k.surface = 'kb_events.payload' AND s.target_id = k.target_id AND j.path = k.path
              UNION ALL SELECT j.unit FROM sensitivity.src_kb_events__metadata s,
                                           sensitivity.jsonb_units(s.doc, s.roots) j
               WHERE k.surface = 'kb_events.metadata' AND s.target_id = k.target_id AND j.path = k.path
              UNION ALL SELECT j.unit FROM sensitivity.src_kb_properties__property_value s,
                                           sensitivity.jsonb_units(s.doc, s.roots) j
               WHERE k.surface = 'kb_properties.property_value' AND s.target_id = k.target_id
                 AND j.path = k.path
          ) u
    ), closed AS (
        INSERT INTO sensitivity.dispositions (finding_id, state)
        SELECT id, 'false_positive'
          FROM units
         GROUP BY id
        HAVING bool_and(unit IS NOT NULL) AND NOT bool_or(card)
        RETURNING 1
    )
    SELECT count(*)::int FROM closed;
$$;

SELECT sensitivity.close_cardless_card_findings();

SELECT declare_migration(
    20261004120000,
    'additive',
    'payment_card v3 (Q51): a new validator, sensitivity.card_valid, requires a card issuer prefix at a length that issuer uses, then Luhn. The detectors validator CHECK admits card_valid; sensitivity.detector_matches gains its arm, its body otherwise 20261002200000''s; payment_card''s row moves to its next version with validator card_valid, recorded in detector_versions by the existing trigger. It is enabled, which turns payment_card back on wherever an operator disabled v2. A new function, sensitivity.close_cardless_card_findings, runs once: each open payment_card finding of an earlier version whose place still holds its unit, and in whose unit the current version finds nothing, gains one false_positive row in sensitivity.dispositions (append-only). Additive: no deployed binary names the sensitivity schema (the grep gate holds it).'
);
