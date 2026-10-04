-- Q51: payment_card v3 requires a card issuer at a length that issuer uses.
--
-- Production's first sweep (2026-10-04) wrote 909 payment_card v2 findings. Measured on a scratch
-- branch of production, none held a match with a card issuer prefix: of 1,186 matches on the text
-- surfaces, 1,103 were 14-digit YYYYMMDDhhmmss migration stamps, and the rest were 14-digit runs
-- beginning 20, epoch milliseconds and 18-digit ids. Each passed Luhn, which one digit run in ten
-- does. Over every block, chunk and event payload, v2 makes 1,235 matches and v3 makes none.
--
-- The pattern and Q42's hex guard stand; only the validator changes. Production disabled v2 after
-- that sweep; v3 is enabled here, so the deploy that ships the fix also turns it on (0.6.0 item 3). v2's findings close as
-- superseded once v3's backfill passes their places (Q27, read in 3b).

-- The issuer table follows ISO/IEC 7812 prefixes as the networks publish them. A prefix outside
-- its lengths is not a card: Luhn alone passes one run of digits in ten.
CREATE FUNCTION sensitivity.card_valid(p_match text) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT CASE
               WHEN d ~ '^4'                                     THEN length(d) IN (13, 16, 19)
               WHEN d ~ '^(5[1-5]|222[1-9]|22[3-9]|2[3-6]|27[01]|2720)' THEN length(d) = 16
               WHEN d ~ '^3[47]'                                 THEN length(d) = 15
               WHEN d ~ '^(6011|64[4-9]|65)'                     THEN length(d) BETWEEN 16 AND 19
               WHEN d ~ '^35(2[89]|[3-8])'                       THEN length(d) BETWEEN 16 AND 19
               WHEN d ~ '^62'                                    THEN length(d) BETWEEN 16 AND 19
               WHEN d ~ '^3(0[0-5]|095|[689])'                   THEN length(d) BETWEEN 14 AND 19
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

-- Findings an earlier version wrote close as false positives, so no operator resolves 909 of them
-- one by one. Q27 carries a disposition across a bump by place and value, so a v3 finding where v2
-- was ruled false would inherit the ruling: a place is closed only when v3 finds nothing in its
-- unit (a jsonb row is read whole, which leaves a finding open rather than closing too much; a
-- unit no longer there has nothing to find). Every
-- place is rescanned by v3's own cursors (D4 rule 2), so a card is found again as a v3 finding. On
-- production this was measured: no v2 finding's content held a match v3 accepts.
INSERT INTO sensitivity.dispositions (finding_id, state)
SELECT f.id, 'false_positive'
  FROM sensitivity.findings f
 WHERE f.detector_id = 'payment_card' AND f.detector_version < 3
   AND NOT EXISTS (SELECT 1 FROM sensitivity.dispositions x WHERE x.finding_id = f.id)
   AND NOT EXISTS (SELECT 1 FROM sensitivity.detector_matches('payment_card', 3, CASE f.surface
           WHEN 'kb_block_content.content' THEN (SELECT s.unit FROM sensitivity.src_kb_block_content__content s WHERE s.target_id = f.target_id)
           WHEN 'kb_chunk_content.content' THEN (SELECT s.unit FROM sensitivity.src_kb_chunk_content__content s WHERE s.target_id = f.target_id)
           WHEN 'kb_chunks.header_path' THEN (SELECT s.unit FROM sensitivity.src_kb_chunks__header_path s WHERE s.target_id = f.target_id)
           WHEN 'kb_resources.title' THEN (SELECT s.unit FROM sensitivity.src_kb_resources__title s WHERE s.target_id = f.target_id)
           WHEN 'kb_resources.origin_uri' THEN (SELECT s.unit FROM sensitivity.src_kb_resources__origin_uri s WHERE s.target_id = f.target_id)
           WHEN 'kb_properties.property_key' THEN (SELECT s.unit FROM sensitivity.src_kb_properties__property_key s WHERE s.target_id = f.target_id)
           WHEN 'kb_edges.label' THEN (SELECT s.unit FROM sensitivity.src_kb_edges__label s WHERE s.target_id = f.target_id)
           WHEN 'kb_citation_audits.reason' THEN (SELECT s.unit FROM sensitivity.src_kb_citation_audits__reason s WHERE s.target_id = f.target_id)
           WHEN 'kb_remote_sources.uri' THEN (SELECT s.unit FROM sensitivity.src_kb_remote_sources__uri s WHERE s.target_id = f.target_id)
           WHEN 'kb_events.payload' THEN (SELECT s.doc::text FROM sensitivity.src_kb_events__payload s WHERE s.target_id = f.target_id)
           WHEN 'kb_events.metadata' THEN (SELECT s.doc::text FROM sensitivity.src_kb_events__metadata s WHERE s.target_id = f.target_id)
           WHEN 'kb_properties.property_value' THEN (SELECT s.doc::text FROM sensitivity.src_kb_properties__property_value s WHERE s.target_id = f.target_id)
       END));

SELECT declare_migration(
    20261004120000,
    'additive',
    'payment_card v3 (Q51): a new validator, sensitivity.card_valid, requires a card issuer prefix at a length that issuer uses, then Luhn. The detectors validator CHECK admits card_valid; sensitivity.detector_matches gains its arm, its body otherwise 20261002200000''s; payment_card''s row moves to version 3 with validator card_valid, recorded in detector_versions by the existing trigger. It is enabled, which turns payment_card back on wherever an operator disabled v2. Every undisposed payment_card finding below version 3 whose unit holds no v3 match gains one false_positive row in sensitivity.dispositions (append-only; v3 rescans every place). Additive: no deployed binary names the sensitivity schema (the grep gate holds it).'
);
