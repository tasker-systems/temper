-- `context_rename` refuses a retired context, the way `context_retire` and `context_restore`
-- already refuse the state they cannot act on.
--
-- THE DEFECT. Retirement (`20260826000110`) made `kb_contexts` a table with two states, and gave
-- every read predicate a floor. Three of the four admin-axis verbs got one too: `context_retire`
-- refuses an already-retired row, `context_restore` refuses an active one, and
-- `reassign_service::admin_reach` floors its reach. `context_rename` did not -- it predates
-- retirement (`20260731000040`) and reads `kb_contexts` by primary key with no state guard, while
-- `caller_administers_context` is `is_active`-BLIND by design, which is exactly what lets `restore`
-- act on a retired row.
--
-- So a caller who administers a retired context could rename it, and `_project_context_renamed`
-- drives `slug` straight from the payload with no floor of its own. The retired row lands on a LIVE
-- address: `UNIQUE (owner_table, owner_id, slug)` is one space shared by active and retired rows
-- (`context_service.rs:1201` states this and warns against "fixing" it), so an invisible row now
-- occupies a name nothing can see holding it. A subsequent `create` under that name silently takes
-- a `-2` suffix against a competitor the caller cannot enumerate.
--
-- The design PR #784 landed states the rule this restores: retired contexts are addressed on the
-- ADMIN axis, never the read axis. Renaming reaches through an admin-axis gate and takes a
-- READ-axis address, which is the one thing that rule forbids.
--
-- WHY HERE AND NOT ONLY IN RUST. Same division `context_rename`'s own header sets out for
-- authorization: the service runs the identical check up front so the caller gets the refusal and
-- no event is appended on the common path, and this guard is the atomic backstop that makes the
-- pre-check advisory. A retirement landing between the service's read and its write is exactly the
-- check-then-act window the SQL-side invariant exists to close.
--
-- ERRCODE P0002 matches `context_retire`/`context_restore`'s state refusals, which
-- `map_context_write_err` already renders as `CONTEXT_REFUSAL` -- a 404 that does not disclose that
-- a retired context sits behind the name, the same non-disclosure #784's refusal parity is about.
--
-- CREATE OR REPLACE, signature and return type unchanged, so a lagging binary keeps working: it
-- calls `context_rename(jsonb, uuid, jsonb, uuid, uuid) RETURNS uuid` exactly as before and gets a
-- uuid, with one more state its call can be refused in.
CREATE OR REPLACE FUNCTION context_rename(p_payload jsonb, p_emitter uuid,
                               p_metadata jsonb DEFAULT '{}'::jsonb,
                               p_invocation uuid DEFAULT NULL,
                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_is_active boolean;
    v_actor uuid;
BEGIN
    -- Existence + current owner + current activation. The owner drives the "administers the
    -- context" gate; the activation flag drives the state refusal directly below. One read, the
    -- shape `context_retire` uses (20260826000120:124-128).
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_rename: context % not found', v_context;
    END IF;

    -- The floor this migration exists for. Before the authorization gate deliberately: whether the
    -- row is renameable at all does not depend on who is asking, and a caller who does not
    -- administer it must still get 42501 rather than learning its retirement state.
    IF NOT v_is_active THEN
        RAISE EXCEPTION 'context_rename: context % is retired', v_context
              USING ERRCODE = 'P0002';
    END IF;

    -- The acting principal IS the emitter; kb_entities.profile_id (NOT NULL) is the human/machine
    -- behind the actor. Authorize that profile, not the emitter entity.
    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_rename: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        -- Context side: the actor must administer the owner (own it directly, or owner/maintainer
        -- on the owning team -- matching `caller_administers_context`).
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_rename: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_rename: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_rename: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    v_ev := _event_append('context_renamed', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_renamed(v_ev, p_payload);
END;
$$;

SELECT declare_migration(
    20260826000130,
    'additive',
    'One CREATE OR REPLACE on context_rename, signature and return type unchanged (jsonb, uuid, jsonb, uuid, uuid -> uuid), adding the is_active state guard that context_retire and context_restore already carry. A binary predating this migration keeps working: it calls the same signature, decodes the same uuid, and every rename it could previously make against an ACTIVE context still succeeds byte-identically. What changes is a refusal that did not exist -- renaming a RETIRED context now raises P0002, which map_context_write_err already renders as the ordinary context refusal. Nothing is dropped and no column, index or constraint moves. Closes the gap left by 20260826000110, which floored the read and write predicates and the retire/restore verbs but not rename: a retired row could be renamed onto a live address inside the shared UNIQUE (owner_table, owner_id, slug) space, where nothing that can see the address can see the row holding it.'
);
