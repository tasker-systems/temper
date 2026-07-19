# Emit a ledger event on Slack disconnect

Task `019f75ec-f82f-73f1-b038-81993e822f5a`. Build/medium. Branch `jct/slack-disconnect-ledger-event`.

Every symbol below was grepped against `main` @ `d37552b8` before this plan was written.

## The event

Name: **`slack_principal_disconnected`**. Classified `category = 'admin'`. NULL-anchored (both
`producing_anchor_table` and `producing_anchor_id`), like every admin act.

Subject is the **profile** being unbound (`AnchorTable::Profiles`). It cannot be the auth-link row:
`AnchorTable` (`payloads.rs:31-50`) has nine variants, `kb_profile_auth_links` is not one of them,
and `as_str` has no `_ =>` arm by design. The Slack principal string rides in the payload.

## Two corrections to the task body

**1. Do NOT add the name to `system.yaml`.** The task body says to. Doing so would trip
`seed_migration_event_types_match_system_yaml` (`bootseed.rs:117-124`), which asserts every
`system.yaml` name also appears in `migrations/20260624000003_canonical_seed.sql` — a **shipped,
applied migration that must not be edited**. The correct precedent is `admin_ledger_opened`: typed,
stamped by its own forward migration, and **absent from `system.yaml`**. `bootseed_publishes_payload_schemas`
(`bootseed.rs:83`) is a **count-only** assertion (`count(*) WHERE payload_schema IS NOT NULL` ==
`TYPED_EVENT_NAMES.len()`), not a set assertion, so a migration-stamped 19th name satisfies it.

**2. `insert_grant`/`delete_grant` already emit.** The task body says they emit nothing; Task 5
changed that. They are the pattern to copy.

## The writer is SQL, not Rust

There is no generic Rust admin-event writer and none should be invented. Task 5's shape is: one
`_admin_<act>()` plpgsql function that performs the state mutation **and** calls `_event_append`
in the same transaction; Rust calls it with a single `query_scalar!`. Only two such call sites exist
(`access_service.rs:296`, `:330`), and there is zero `INSERT INTO kb_events` in `temper-services/src`.

This also solves capture-before-delete for free: `DELETE ... RETURNING profile_id` feeds the event.

## Beat A — substrate (must land before Beat B; B inlines A's snapshot verbatim)

`crates/temper-substrate/src/payloads.rs`:
- Add `SlackPrincipalDisconnected` next to the admin-ledger payload block (`:911-968`). Derives
  exactly: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]` +
  `#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]`.
  Fields: `subject_table: AnchorTable`, `subject_id: Uuid`, `slack_principal_id: String`,
  `disconnected_by: ProfileId`.
  **No** `resource_id` / `block_id` / `edge_id` / `owner` — the four banned keys.
- `TYPED_EVENT_NAMES` (`:971`): `[&str; 18]` → `[&str; 19]`, append the name.
- `ADMIN_EVENT_NAMES` (`:997`): `[&str; 3]` → `[&str; 4]`, append the name. **The task body omits
  this const.** Without it a reseed loses the `category = 'admin'` classification.
- `verify_ledger_roundtrip` (`:999-1065`): add an arm. The `_ => {}` at `:1052` means omission is
  **silent**, not a compile error.

`crates/temper-substrate/src/events.rs`:
- `EventKind` variant `SlackPrincipalDisconnected` (`:33-88`).
- `as_canonical_name` arm (`:90`) — compiler-forced, will not build without it.
- `from_canonical_name` arm (`:123`) — **NOT compiler-forced** (`_ => return None` at `:157`).
  Omitting it is a hard replay failure at `replay.rs:351-352`. This is the single highest-risk line
  in the change.

`crates/temper-substrate/src/replay.rs` — both matches are exhaustive/compiler-forced:
- Content-sidecar arm (`:158-202`): add to the `=> None` group beside the admin events at `:199-202`.
- Projection-walk no-op (`:353`, admin arm at `:533-538`): add to the `=> {}` group.
- Check the second sidecar match at `:246`.

`crates/temper-substrate/tests/payload_schema.rs`:
- Add `check::<p::SlackPrincipalDisconnected>("slack_principal_disconnected");`.
- Regenerate: `UPDATE_SCHEMA=1 cargo make test-schema`. **Package-scoped only** —
  `-p temper-substrate --features scenario-schema`, never `--workspace` (feature unification changes
  the emitted schema).
- Snapshot lands at `crates/temper-substrate/tests/fixtures/payloads/slack_principal_disconnected.v1.schema.json`
  (18 files → 19). `snapshot_files_cover_exactly_the_typed_names` asserts set equality with
  `TYPED_EVENT_NAMES`.

## Beat B — migration `20260719000020_slack_disconnect_event.sql`

Slot verified against **prod** `_sqlx_migrations`: tail is `20260718000030`, all `success = t`,
identical to local. `…000010` is deliberately left free for sibling sessions.

Three things, in one file:
1. `INSERT INTO kb_event_types (name, payload_schema, schema_version)` with Beat A's snapshot pasted
   **byte-for-byte** inside `$JS$…$JS$::jsonb`, `ON CONFLICT (name) DO UPDATE`. Template:
   `migrations/20260717000010_admin_event_types.sql`.
2. `UPDATE kb_event_types SET category = 'admin' WHERE name = 'slack_principal_disconnected';`
   **The task body omits this.** `category` defaults to `'cognition'` (`20260718000020:47`) and the
   trail firewall is an allowlist (`et.category = 'cognition'`), so an unstamped admin type passes
   filter B and leans entirely on filter A (anchor nullity).
3. `CREATE FUNCTION _admin_slack_disconnected(p_emitter uuid, p_slack_principal_id text,
   p_disconnected_by uuid, p_correlation uuid DEFAULT NULL) RETURNS boolean` —
   `DELETE FROM kb_profile_auth_links WHERE auth_provider = 'slack' AND auth_provider_user_id = p_slack_principal_id
   RETURNING profile_id INTO v_profile;` then, **only if a row was deleted**, `PERFORM _event_append(...)`
   with `p_anchor_table => NULL, p_anchor_id => NULL` positionally and
   `p_references => jsonb_build_array(jsonb_build_object('rel','subject','target',
   jsonb_build_object('kind','kb_profiles','id',v_profile)))`.
   Emit-only-if-deleted mirrors `_admin_grant_revoked`: a no-op disconnect is not an admin act and
   `kb_events` is append-only, so a spurious row is immortal.

## Beat C — service wiring

`crates/temper-services/src/services/slack_disconnect_service.rs`:
- Add `pub actor: ProfileId` to `DisconnectRequest` (`:45-52`). Authentication is already enforced
  upstream on both arms; the actor is simply **discarded at the service boundary** today
  (`disconnect_me:89` never passes it; `admin_disconnect_slack_principal:207` drops it after gating).
- Replace the raw `DELETE FROM kb_profile_auth_links` at `:127-139` with a `query_scalar!` call to
  `_admin_slack_disconnected`, taking `&mut *tx` so it joins the existing transaction that commits
  at `:157`. Return value replaces `was_linked`.
- Resolve the emitter in the caller: `temper_substrate::writes::resolve_emitter(pool, actor, "web")`.
- The Rust wrapper performs **no** authorization — gating stays where it is.

`crates/temper-api/src/handlers/slack_disconnect.rs`: pass `actor` on both arms — `profile_id`
(already in scope at `:72`) on self-serve, `ProfileId::from(auth.0.profile.id)` on admin.

On self-serve actor == subject; on admin they differ. **Distinguishing those two is most of why this
event is worth having.**

## Beat D — admin ledger read surface

`crates/temper-services/src/services/admin_ledger_service.rs`:
- `ADMIN_EVENT_TYPES` (`:54`): add the name.
- `readable_event_types` (`:56-99`): add an arm, or the fail-closed default (absence ⇒ admin-only)
  silently applies. **Open policy question — see below.**

`crates/temper-services/tests/admin_ledger_test.rs`: add to `ADMIN_EVENT_TYPES_FOR_TEST` (`:36-38`),
or the banned-key corpus scan `no_admin_payload_spells_a_trail_matched_key` (`:628`) never sees the
new type. The coupling is manual and the const's own doc-comment names this test as the drift-catcher.

> **DECIDED 2026-07-19 — admin-only, deliberately.** No arm is added; the fail-closed default *is*
> the policy. An admin disconnect may be one step in an offboarding playbook, and the ordering is not
> guaranteed to put temper last — so letting the subject read it would leak the **timing** of an
> in-flight administrative action (most consequentially a termination) before it completes. Self-serve
> stays readable via `list_by_actor` (actor == subject there, so no third party is concealed). The
> line the policy draws is **actor ≠ subject**.
>
> Rationale, enforcement, and revisiting conditions:
> [docs/decisions/2026-07-19-admin-disconnect-is-not-subject-readable.md](../../decisions/2026-07-19-admin-disconnect-is-not-subject-readable.md).
> Because absence-of-arm is doing policy work, that doc is what distinguishes "deliberately absent"
> from "nobody wrote it yet." A future arm proposal must revisit it first.

## Beat E — tests (red before green, each one)

1. **Service unit test** (`slack_disconnect_service.rs` tests mod, `#[sqlx::test]`): a disconnect
   writes **exactly one** `slack_principal_disconnected` row naming actor and subject. Extend
   `disconnect_deletes_link_grant_and_intents_together` (`:327`) with a fourth assertion.
2. **Idempotence**: a second disconnect writes **zero** further events (`disconnecting_twice_is_not_an_error`, `:418`).
3. **Banned-key assertion** — asserted, not assumed. Covered by adding the name to
   `ADMIN_EVENT_TYPES_FOR_TEST` so the existing corpus scan includes it.
4. **Replay**: a database containing the new event replays successfully. This is the test that
   catches a missing `from_canonical_name` arm; without it that omission ships silently.
5. **E2E** (`tests/e2e/tests/slack_link_test.rs`): admin-initiated disconnect records actor ≠ subject.

Each gate must be **verified red against the pre-fix commit** before being made green. A gate can be
unreachable in exactly the way the code is wrong — PR #488's lesson.

## Not in scope

`audit-grant-sinks.sh` needs **no** change: it greps only `insert_grant(` and
`INSERT INTO kb_access_grants`. This work touches neither, so it is outside the tripwire's scope.
Its own header flags a known blind spot — SQL-side grant writes are invisible to it — which is a
real gap but a **separate** task, not this PR's narrative.

## Gates

- `cargo make test-schema` green (package-scoped).
- `cargo make check` (offline sqlx — the honest local probe).
- `cargo make prepare-services` after the new `query_scalar!` lands, then `cargo make check` again.
- `cargo make test-db`, `cargo make test-e2e`.
