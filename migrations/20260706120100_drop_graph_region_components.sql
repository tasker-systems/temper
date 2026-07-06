-- A3: components are a region's PARENT grain (kb_cogmap_regions.component_id), not
-- sub-clusters of it, and this fn returned all cogmap/lens components (no reg.id tie).
-- The R3 slice no longer surfaces components. Drop the dead function.
DROP FUNCTION IF EXISTS graph_region_components(uuid, uuid);
