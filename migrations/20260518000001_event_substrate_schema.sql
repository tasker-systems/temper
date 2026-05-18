-- Event substrate v1 schema.
-- See docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md.

CREATE SCHEMA event_substrate;

CREATE TYPE event_substrate.porosity AS ENUM ('access', 'attention');

CREATE TABLE event_substrate.profiles (
    id          uuid PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    name        text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE event_substrate.entities (
    id          uuid PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    profile_id  uuid NOT NULL REFERENCES event_substrate.profiles(id),
    name        text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX entities_profile_id_idx
    ON event_substrate.entities(profile_id);

CREATE TABLE event_substrate.topics (
    id          uuid PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    fqdn        text NOT NULL UNIQUE,
    parent_id   uuid REFERENCES event_substrate.topics(id),
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE event_substrate.scopes (
    id          uuid PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    name        text NOT NULL UNIQUE,
    porosity    event_substrate.porosity NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE event_substrate.event_types (
    id              uuid PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    name            varchar(128) NOT NULL UNIQUE,
    description     text,
    is_deprecated   boolean NOT NULL DEFAULT false,
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE event_substrate.events (
    id                   uuid PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    event_type_id        uuid NOT NULL REFERENCES event_substrate.event_types(id),
    emitter_entity_id    uuid NOT NULL REFERENCES event_substrate.entities(id),
    topic_id             uuid NOT NULL REFERENCES event_substrate.topics(id),
    scope_id             uuid NOT NULL REFERENCES event_substrate.scopes(id),
    payload              jsonb NOT NULL,
    metadata             jsonb NOT NULL DEFAULT '{}'::jsonb,
    "references"         jsonb NOT NULL DEFAULT '[]'::jsonb,
    correlation_id       uuid NOT NULL,
    occurred_at          timestamptz NOT NULL,
    recorded_at          timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX events_topic_recorded_idx
    ON event_substrate.events(topic_id, recorded_at DESC);
CREATE INDEX events_event_type_recorded_idx
    ON event_substrate.events(event_type_id, recorded_at DESC);
CREATE INDEX events_emitter_recorded_idx
    ON event_substrate.events(emitter_entity_id, recorded_at DESC);
CREATE INDEX events_correlation_idx
    ON event_substrate.events(correlation_id);
CREATE INDEX events_references_gin_idx
    ON event_substrate.events USING gin ("references" jsonb_path_ops);

-- Append-only enforcement.
CREATE OR REPLACE FUNCTION event_substrate.events_append_only()
RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'event ledger is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER events_no_update_or_delete
    BEFORE UPDATE OR DELETE ON event_substrate.events
    FOR EACH ROW EXECUTE FUNCTION event_substrate.events_append_only();

CREATE TABLE event_substrate.concepts (
    id                        uuid PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    current_definition        text NOT NULL,
    current_elaboration       text,
    scope_id                  uuid NOT NULL REFERENCES event_substrate.scopes(id),
    topic_id                  uuid NOT NULL REFERENCES event_substrate.topics(id),
    created_by_event_id       uuid NOT NULL REFERENCES event_substrate.events(id),
    last_event_id             uuid NOT NULL REFERENCES event_substrate.events(id),
    latest_event_recorded_at  timestamptz NOT NULL
);

CREATE INDEX concepts_scope_topic_idx
    ON event_substrate.concepts(scope_id, topic_id);
CREATE INDEX concepts_created_by_event_idx
    ON event_substrate.concepts(created_by_event_id);
CREATE INDEX concepts_last_event_idx
    ON event_substrate.concepts(last_event_id);
