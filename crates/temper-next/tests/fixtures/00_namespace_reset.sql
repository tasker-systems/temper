-- TEST-ONLY: owns + resets the isolated temper_next proving-ground namespace. Production never
-- runs this (the canonical baseline migration builds public under the default search_path). The
-- harness injects search_path=temper_next,public via PGOPTIONS when loading the baseline body files,
-- so this reset needs no SET — the DROP/CREATE are fully qualified.
DROP SCHEMA IF EXISTS temper_next CASCADE;
CREATE SCHEMA temper_next;
