-- B-tree expression indexes for sortable/filterable managed_meta keys.
CREATE INDEX idx_manifests_managed_stage
    ON kb_resource_manifests ((managed_meta->>'temper-stage'));
CREATE INDEX idx_manifests_managed_seq
    ON kb_resource_manifests (((managed_meta->>'temper-seq')::bigint));
CREATE INDEX idx_manifests_managed_mode
    ON kb_resource_manifests ((managed_meta->>'temper-mode'));
CREATE INDEX idx_manifests_managed_effort
    ON kb_resource_manifests ((managed_meta->>'temper-effort'));
CREATE INDEX idx_manifests_managed_doc_type
    ON kb_resource_manifests ((managed_meta->>'temper-type'));

-- GIN with jsonb_path_ops for future ad-hoc containment queries.
CREATE INDEX idx_manifests_managed_meta_gin
    ON kb_resource_manifests USING gin (managed_meta jsonb_path_ops);

-- Pre-joined view that every vault browse query uses.
-- Encapsulates the 6-table join so service code only adds
-- WHERE / ORDER BY / LIMIT / OFFSET against flat columns.
CREATE VIEW vault_resources_browse AS
SELECT r.id,
       r.kb_context_id,
       r.kb_doc_type_id,
       r.origin_uri,
       r.title,
       r.slug,
       r.originator_profile_id,
       r.owner_profile_id,
       r.is_active,
       r.created,
       r.updated,
       c.name                                      AS context_name,
       dt.name                                     AS doc_type_name,
       c.kb_owner_table,
       c.kb_owner_id,
       COALESCE(t.slug, '')                        AS team_slug,
       m.managed_meta->>'temper-stage'             AS stage,
       (m.managed_meta->>'temper-seq')::bigint     AS seq,
       m.managed_meta->>'temper-mode'              AS mode,
       m.managed_meta->>'temper-effort'            AS effort
  FROM kb_resources r
  JOIN kb_contexts c   ON c.id  = r.kb_context_id
  JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
  JOIN kb_profiles p   ON p.id  = r.owner_profile_id
  LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
  LEFT JOIN kb_teams t ON c.kb_owner_table = 'kb_teams' AND t.id = c.kb_owner_id
 WHERE r.is_active = true;
