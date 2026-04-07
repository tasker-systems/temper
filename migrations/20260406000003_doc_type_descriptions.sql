-- Add description column to kb_doc_types for agent discoverability
ALTER TABLE kb_doc_types ADD COLUMN description TEXT;

-- Populate descriptions for the canonical six doc types
UPDATE kb_doc_types SET description = CASE name
    WHEN 'task' THEN 'Task definitions, acceptance criteria, and tracking'
    WHEN 'goal' THEN 'Goal definitions, progress tracking, and strategic objectives'
    WHEN 'session' THEN 'Session notes capturing what happened during a working session'
    WHEN 'research' THEN 'Investigation findings, analysis, and evaluation results'
    WHEN 'decision' THEN 'Decision records capturing choices, rationale, and trade-offs'
    WHEN 'concept' THEN 'Ideas, patterns, and cross-cutting themes that span projects'
    ELSE NULL
END;
