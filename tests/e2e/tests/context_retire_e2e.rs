#![cfg(feature = "test-db")]
//! Context-retirement e2e: the reversal itself, the mangled ref that makes it operable, and the
//! authorization arms `context_delete_e2e.rs` (PR #777) never exercised.
//!
//! `DELETE /api/contexts/{id}` used to be the one context mutation with no `is_active` to flip
//! back — a hard delete, refused with `409` while anything was still homed in the context.
//! `20260826000110_context_retirement.sql` supersedes that: `kb_contexts` is a replay INPUT table
//! restored verbatim (`crates/temper-substrate/src/replay.rs:101-125`) and both context
//! projectors RAISE on a missing row, so a hard delete broke replay for any context that was ever
//! renamed or reassigned. `retire` now flips `kb_contexts.is_active` to `false` and mangles the
//! slug to `<slug>-retired` (suffixed if that was taken) instead of deleting the row — every
//! clause of the old module doc is now false. **There is no dependents guard.** Retiring a
//! context that still homes a live resource is the whole point: the container disappears from the
//! read axis, the resource does not.
//!
//! `restore` is the reverse: `is_active` flips back to `true` and the slug is RE-DERIVED from the
//! untouched `name`, not recovered from whatever `retire` mangled it to — so the restored address
//! can differ from the one the caller retired under (it does not, when nothing else raced for the
//! original slug in between).
//!
//! Ingest uses empty content (no body → no embed) so this runs on plain `cargo make test-e2e`
//! without `test-embed`.
//!
//! Modeled on `context_rename_e2e.rs` (its `provision` / `root_bootstrap_first_admin` /
//! `grant_context_read` idiom) and the delete suite this file replaces (`provision`,
//! `root_bootstrap_first_admin`, the HTTP status helper, and the `cli_delete` spawn helper are all
//! reused, adapted only where the verb or its wire shape changed).

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

/// Provision a profile by hitting an authed endpoint (auto-provision on first request), then
/// approve it — a fresh principal is born Denied (D11), and this test exercises the retire/restore
/// endpoints' own gate, not the front door.
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    let pid: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    common::approve(&app.pool, pid).await;
    pid
}

/// The irreducible operator root step: configure gating + mint first admin.
async fn root_bootstrap_first_admin(pool: &sqlx::PgPool, admin_id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(pool)
        .await
        .expect("gating");
    common::approved_admin(pool, admin_id).await;
}

/// Seed a representative non-admin reader: an explicit profile-anchored read grant on the
/// context. Copied verbatim from `context_rename_e2e.rs`'s own helper of the same name — the
/// incumbent fixture for "a caller who reads but does not administer", chosen there (and here)
/// because it needs no team setup: the context stays personal and only this row changes the
/// answer. `granted_by_profile_id` is **NOT NULL** on `kb_access_grants`.
async fn grant_context_read(pool: &sqlx::PgPool, context: Uuid, principal: Uuid, granted_by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, \
              granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(context)
    .bind(principal)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("seed context read-grant");
}

/// `POST /api/ingest` homed at `context_ref`, as `token`. Returns `(status, resource_id, body)` —
/// the body rides along so a caller can print the server's actual error on an unexpected status
/// instead of just the bare status code.
async fn ingest(
    app: &common::E2eTestApp,
    token: &str,
    context_ref: &str,
    slug: &str,
) -> (StatusCode, Option<Uuid>, Value) {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": format!("ctx-retire {slug}"),
            "origin_uri": format!("test://context-retire-e2e/{}", Uuid::new_v4()),
            "context_ref": context_ref,
            "doc_type_name": "research",
            "slug": slug,
            "content": "",
        }))
        .send()
        .await
        .expect("ingest request");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let id = body["id"].as_str().and_then(|s| s.parse().ok());
    (status, id, body)
}

/// `GET /api/contexts/{id}` status — the read oracle both the round-trip and the authorization
/// tests check against.
async fn context_get_status(app: &common::E2eTestApp, token: &str, context_id: Uuid) -> StatusCode {
    app.reqwest_client
        .get(app.url(&format!("/api/contexts/{context_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("context get")
        .status()
}

/// Wire-level status + body of `DELETE /api/contexts/{context}` (retire) as `token`. Renamed from
/// the old suite's `delete_status`: the HTTP verb stays `DELETE`, but what it does is `retire`.
async fn retire_status(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
) -> (StatusCode, Value) {
    let resp = app
        .reqwest_client
        .delete(app.url(&format!("/api/contexts/{context_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("retire request");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// Wire-level status + body of `POST /api/contexts/{context}/restore` as `token`.
async fn restore_status(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
) -> (StatusCode, Value) {
    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/contexts/{context_id}/restore")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("restore request");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// Run the real `temper` binary's `context delete <ref>` — the operator-facing verb for retire
/// (the CLI verb stays `delete`; only the service function is named `retire`, per spec §2.6).
/// Returns `(success, combined output)`.
async fn cli_delete(app: &common::E2eTestApp, token: &str, context_id: Uuid) -> (bool, String) {
    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig");
    let config_path = app.vault_dir.path().join("test-temper-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config");

    let bare_uuid = context_id.to_string();
    let out = common::run_temper_cli_with_token(
        &app.base_url(),
        token,
        &config_path,
        &["context", "delete", &bare_uuid],
    )
    .await
    .expect("spawn temper");

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = if stdout.trim().is_empty() {
        stderr
    } else {
        stdout
    };
    (out.status.success(), combined)
}

/// Run the real `temper` binary's `context restore <ref>`, where `context_ref` is whatever string
/// the caller hands it — a bare UUID, or the mangled `owner_ref/slug` that `delete` printed.
/// Returns `(success, combined output)`.
async fn cli_restore(app: &common::E2eTestApp, token: &str, context_ref: &str) -> (bool, String) {
    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig");
    let config_path = app.vault_dir.path().join("test-temper-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config");

    let out = common::run_temper_cli_with_token(
        &app.base_url(),
        token,
        &config_path,
        &["context", "restore", context_ref],
    )
    .await
    .expect("spawn temper");

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = if stdout.trim().is_empty() {
        stderr
    } else {
        stdout
    };
    (out.status.success(), combined)
}

/// The headline inversion: a context still homing a live resource used to be refused with `409`;
/// now retiring it SUCCEEDS, the resource survives untouched, the context vanishes from the
/// ordinary read axis, and `restore` brings it back to exactly the address it started at.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retiring_an_attached_context_succeeds_and_restore_reverses_it(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let context = app
        .client
        .contexts()
        .create("Retire Me", None)
        .await
        .expect("admin creates the context");
    let context_ref = format!("{}/{}", context.owner_ref, context.slug);

    let (ing, resource_id, ing_body) = ingest(&app, &app.token, &context_ref, "attached").await;
    assert_eq!(
        ing,
        StatusCode::OK,
        "admin authors into their own context: {ing_body:?}"
    );
    let resource_id = resource_id.expect("ingest returns the new resource id");

    // ── the inversion: retiring a context that still homes a live resource SUCCEEDS ──────────
    let outcome = app
        .client
        .contexts()
        .delete(*context.id)
        .await
        .expect("retiring an attached context is the whole point of retirement, not a 409");
    assert_eq!(
        *outcome.context_id, *context.id,
        "the outcome names the same context"
    );
    assert_eq!(
        outcome.name, "Retire Me",
        "retirement never touches the display name"
    );
    assert_eq!(
        outcome.slug, "retire-me-retired",
        "the mangled address — the freed original stays free for reuse"
    );
    assert_eq!(
        outcome.context_ref,
        format!("{}/retire-me-retired", context.owner_ref),
        "the outcome carries the composed mangled ref"
    );

    // The resource survives, and its owner can still read it — nothing was stranded by the
    // container's disappearance.
    let show = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("show request")
        .status();
    assert_eq!(
        show,
        StatusCode::OK,
        "the homed resource survives its container's retirement, and stays readable by its owner"
    );

    // The context is gone from the ordinary listing — the read axis, not the admin axis.
    let visible = app.client.contexts().list().await.expect("list contexts");
    assert!(
        !visible.iter().any(|c| *c.id == *context.id),
        "a retired context is invisible on the read axis by construction"
    );
    let retired_listing = app
        .client
        .contexts()
        .list_retired()
        .await
        .expect("list retired contexts");
    assert!(
        retired_listing.iter().any(|c| *c.id == *context.id),
        "but it IS reachable on the admin axis, which is what makes restore possible"
    );

    // ── restore reverses it ────────────────────────────────────────────────────────────────
    let restored = app
        .client
        .contexts()
        .restore(*context.id)
        .await
        .expect("the admin restores their own retired context");
    assert_eq!(*restored.context_id, *context.id);
    assert_eq!(restored.name, "Retire Me");
    assert_eq!(
        restored.slug, "retire-me",
        "restore re-derives the ORIGINAL slug from the untouched name, not the mangled one"
    );
    assert!(
        !restored.slug_changed,
        "nothing else took the original slug while the context was retired"
    );
    assert_eq!(
        restored.context_ref, context_ref,
        "restore lands back on the exact address the context started with"
    );

    let visible_again = app.client.contexts().list().await.expect("list contexts");
    assert!(
        visible_again.iter().any(|c| *c.id == *context.id),
        "restore returns the context to the ordinary read axis"
    );
    assert_eq!(
        context_get_status(&app, &app.token, *context.id).await,
        StatusCode::OK,
        "the restored context is directly readable again, without the admin-axis fallback"
    );
}

/// The mangled ref is what makes retirement reversible *in practice*: once a context is retired
/// its original `@owner/slug` no longer resolves at all, so the ref `delete` prints in its
/// response is the only address an operator has left. This proves that ref reaches the CLI
/// caller intact and that feeding it back to `context restore`, verbatim, actually works — not
/// just the bare UUID the other test exercises.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_mangled_ref_the_cli_prints_is_what_restore_accepts(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let context = app
        .client
        .contexts()
        .create("Mangled Ref Target", None)
        .await
        .expect("admin creates the context");
    let original_ref = format!("{}/{}", context.owner_ref, context.slug);

    // Retire through the CLI — a real operator's view of the outcome, JSON-rendered because
    // stdout is piped (non-TTY).
    let (cli_ok, cli_out) = cli_delete(&app, &app.token, *context.id).await;
    assert!(
        cli_ok,
        "the CLI `context delete` (retire) must succeed: {cli_out}"
    );
    let outcome: Value = serde_json::from_str(&cli_out)
        .unwrap_or_else(|e| panic!("CLI printed JSON: {e}: {cli_out}"));
    let printed_ref = outcome["context_ref"]
        .as_str()
        .expect("outcome carries context_ref")
        .to_string();
    assert_eq!(
        printed_ref,
        format!("{}/mangled-ref-target-retired", context.owner_ref),
        "the CLI surfaces the server's mangled ref verbatim"
    );
    assert_ne!(
        printed_ref, original_ref,
        "the printed ref is the MANGLED one, not the address the context started with"
    );

    // The original ref is now dead — the whole reason the mangled ref matters.
    let (stale_status, _, _) = ingest(&app, &app.token, &original_ref, "via-stale-ref").await;
    assert_eq!(
        stale_status,
        StatusCode::NOT_FOUND,
        "the original @owner/slug no longer resolves once the context is retired"
    );

    // Feed EXACTLY that printed ref to `context restore` — the round trip that makes retirement
    // reversible for an operator, not only through a raw id over the API.
    let (restore_ok, restore_out) = cli_restore(&app, &app.token, &printed_ref).await;
    assert!(
        restore_ok,
        "the CLI restore must accept the exact ref `delete` printed: {restore_out}"
    );
    let restored: Value = serde_json::from_str(&restore_out)
        .unwrap_or_else(|e| panic!("CLI printed JSON: {e}: {restore_out}"));
    assert_eq!(
        restored["context_ref"].as_str().expect("context_ref"),
        original_ref,
        "restore re-derives the ORIGINAL address from the untouched name"
    );

    let visible = app.client.contexts().list().await.expect("list contexts");
    assert!(
        visible.iter().any(|c| *c.id == *context.id),
        "the context is genuinely back on the read axis"
    );
}

/// The authorization arms PR #777 never exercised: both of its tests provisioned an instance
/// admin via `root_bootstrap_first_admin`, so `retire`/`restore`'s gate (`ContextAdminAuthority`,
/// the same two-dialect gate `rename` uses) was never tried against anyone else.
///
/// A caller who reads the context but does not administer it gets `403`, never the
/// existence-hiding `404` — they already know the row exists. A caller who cannot see it at all
/// gets the uniform `404`, indistinguishable from a context that never existed: no existence
/// oracle, proved here by comparing the refusal body for a REAL (but invisible) context against
/// the refusal for a random UUID.
///
/// ⚠️ **The `403` arm is reachable on `restore` only while the context is still ACTIVE.**
/// `ContextAdminAuthority`'s `ReadOnly` arm delegates entirely to `context_visible_to`
/// (`context_service::context_visible`), and `20260826000110_context_retirement.sql` floors
/// EVERY arm of that predicate — including the explicit-read-grant arm — on `is_active`. Once a
/// context is actually retired, a reader who could see it a moment ago resolves `Invisible`, not
/// `ReadOnly`, so `restore` on an ALREADY-retired context answers the reader with `404`, not
/// `403`. That is not a bug: `authorize()` runs before `restore`'s own "is this even retired?"
/// check, so the `403` fires from a still-active context's authorization the same way `retire`'s
/// does, and the test below pins that transition explicitly rather than eliding it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_authorization_arms_hold_for_both_retire_and_restore(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    let reader_token = common::generate_second_user_jwt();
    let reader_id = provision(&app, &reader_token).await;
    let stranger_token = common::generate_third_user_jwt();
    provision(&app, &stranger_token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    // The admin's personal context; the reader gets an explicit read grant, the stranger nothing.
    let context = app
        .client
        .contexts()
        .create("Guarded", None)
        .await
        .expect("admin creates context");
    grant_context_read(&pool, *context.id, reader_id, admin_id).await;

    // ── the precondition the whole split rests on ─────────────────────────────────────────
    assert_eq!(
        context_get_status(&app, &reader_token, *context.id).await,
        StatusCode::OK,
        "fixture precondition: the reader can read this context"
    );
    assert_eq!(
        context_get_status(&app, &stranger_token, *context.id).await,
        StatusCode::NOT_FOUND,
        "fixture precondition: the stranger cannot see this context at all"
    );

    // ── retire ─────────────────────────────────────────────────────────────────────────────
    let (reader_retire_status, _) = retire_status(&app, &reader_token, *context.id).await;
    assert_eq!(
        reader_retire_status,
        StatusCode::FORBIDDEN,
        "a reader who does not administer the context is refused with 403, not the \
         existence-hiding 404"
    );

    let (stranger_retire_status, stranger_retire_body) =
        retire_status(&app, &stranger_token, *context.id).await;
    assert_eq!(
        stranger_retire_status,
        StatusCode::NOT_FOUND,
        "a non-reader gets the existence-hiding 404 — retire is not an oracle either"
    );
    let (missing_retire_status, missing_retire_body) =
        retire_status(&app, &stranger_token, Uuid::new_v4()).await;
    assert_eq!(missing_retire_status, StatusCode::NOT_FOUND);
    assert_eq!(
        stranger_retire_body, missing_retire_body,
        "a real context the stranger cannot see and a context that never existed render \
         byte-identical refusals — no existence oracle"
    );

    // Nothing changed: three refused attempts, zero writes.
    let still_active: bool = sqlx::query_scalar("SELECT is_active FROM kb_contexts WHERE id = $1")
        .bind(*context.id)
        .fetch_one(&pool)
        .await
        .expect("existence check");
    assert!(still_active, "a refused retire changes nothing");

    // ── restore, BEFORE the context is actually retired ───────────────────────────────────
    // See the module doc's ⚠️: `authorize()` runs before the "is this retired?" check, so the
    // same two dialects fire here even though `restore`'s own guard would otherwise 404 on an
    // active context for an unrelated reason.
    let (reader_restore_status, _) = restore_status(&app, &reader_token, *context.id).await;
    assert_eq!(
        reader_restore_status,
        StatusCode::FORBIDDEN,
        "the same reader is refused restore with 403 too — read-without-administer is refused \
         identically on both verbs"
    );

    let (stranger_restore_status, stranger_restore_body) =
        restore_status(&app, &stranger_token, *context.id).await;
    assert_eq!(stranger_restore_status, StatusCode::NOT_FOUND);
    let (missing_restore_status, missing_restore_body) =
        restore_status(&app, &stranger_token, Uuid::new_v4()).await;
    assert_eq!(missing_restore_status, StatusCode::NOT_FOUND);
    assert_eq!(
        stranger_restore_body, missing_restore_body,
        "restore's refusal is just as uniform as retire's"
    );

    // ── the transition the ⚠️ above documents: retire for real, then re-probe ─────────────
    app.client
        .contexts()
        .delete(*context.id)
        .await
        .expect("the admin retires the context for real");

    let (reader_restore_after, _) = restore_status(&app, &reader_token, *context.id).await;
    assert_eq!(
        reader_restore_after,
        StatusCode::NOT_FOUND,
        "once genuinely retired, the read-grant no longer resolves the reader as ReadOnly — \
         context_visible_to floors on is_active for every arm, so the reader is now Invisible \
         and gets the uniform 404, not 403"
    );
    let (stranger_restore_after, _) = restore_status(&app, &stranger_token, *context.id).await;
    assert_eq!(
        stranger_restore_after,
        StatusCode::NOT_FOUND,
        "the stranger was always Invisible and stays that way"
    );
}

/// Wire-level status + body of `POST /api/contexts/{context}/rename` as `token`.
async fn rename_status(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    name: &str,
) -> (StatusCode, Value) {
    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/contexts/{context_id}/rename")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .expect("rename request");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// A retired context cannot be renamed — and that closes an address it could otherwise occupy
/// where nothing able to see the address can see the row holding it.
///
/// Retirement floored both read predicates, both write predicates, `retire`, `restore` and
/// `reassign`. `rename` predates it (`20260731000040`) and reads `kb_contexts` by primary key with
/// no state guard, while `caller_administers_context` is `is_active`-blind **by design** — that
/// blindness is what lets `restore` act on a retired row at all. So the admin axis reached through
/// and took a **read-axis address**: `UNIQUE (owner_table, owner_id, slug)` is one space shared by
/// active and retired rows, so an invisible row parks on a live slug and the next `create` under
/// that name is silently suffixed against a competitor nobody can enumerate.
///
/// **Three assertions, and each covers a different arm.**
/// 1. The *active* rename first, so a broken fixture cannot masquerade as the refusal under test.
/// 2. A rename to a **new** name after retirement — the path that reaches `context_rename`'s
///    in-transaction guard.
/// 3. A rename to the name the context **already has** — the idempotent no-op, which returns from
///    the service before the write function is ever called. Only the service-side floor covers it,
///    which is why the guard is in both places rather than SQL alone.
///
/// The refusal is `CONTEXT_REFUSAL`, byte-identical to a context that is not there: telling an
/// administrator "retired" where a miss says "not found" would hand back the one bit PR #784's
/// refusal parity exists to withhold.
///
/// **What this does NOT cover, stated so a pass is not read as more than it is.** It witnesses the
/// service-side floor only. Bite-probed both ways: deleting the Rust check leaves arm 2 refused by
/// `context_rename`'s guard but turns arm 3 into a `200` disclosing the retired context's mangled
/// slug, and deleting the SQL guard while keeping the Rust check leaves this test **green**. The
/// SQL half exists for the check-then-act window — a retirement landing between the service's read
/// and its write — and nothing here provokes that interleaving, so it is covered by construction
/// and by the pattern `context_rename`'s authorization gate already sets, not by an assertion.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_retired_context_cannot_be_renamed_onto_a_live_address(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let context = app
        .client
        .contexts()
        .create("Rename Me", None)
        .await
        .expect("admin creates the context");

    // 1. NON-VACUITY: while active, this exact call succeeds and moves the address.
    let (ok, body) = rename_status(&app, &app.token, *context.id, "Renamed Already").await;
    assert_eq!(ok, StatusCode::OK, "an active context renames: {body:?}");
    assert_eq!(
        body["slug"], "renamed-already",
        "and the address moved: {body:?}"
    );

    let (retired, _) = retire_status(&app, &app.token, *context.id).await;
    assert_eq!(retired, StatusCode::OK, "retire the context");

    // 2. The guarded path: a rename to a NEW name reaches `context_rename`.
    let (refused, body) = rename_status(&app, &app.token, *context.id, "Taking A Live Slug").await;
    assert_eq!(
        refused,
        StatusCode::NOT_FOUND,
        "a retired context must not be renameable onto a live address: {body:?}"
    );

    // 3. The arm that never reaches the SQL guard: renaming to the name it already carries is an
    //    idempotent no-op that returns `Ok` from the service.
    let (noop, body) = rename_status(&app, &app.token, *context.id, "Renamed Already").await;
    assert_eq!(
        noop,
        StatusCode::NOT_FOUND,
        "the idempotent no-op arm must refuse too — it returns before the write function: {body:?}"
    );

    // The address the rename tried to take is genuinely free: a new context claims it unsuffixed.
    let fresh = app
        .client
        .contexts()
        .create("Taking A Live Slug", None)
        .await
        .expect("the address was never occupied");
    assert_eq!(
        fresh.slug, "taking-a-live-slug",
        "no retired row is squatting the slug, so no -2 suffix"
    );

    // And the retirement itself is intact — refusing the rename did not disturb the row.
    let restored = app
        .client
        .contexts()
        .restore(*context.id)
        .await
        .expect("restore still works");
    assert_eq!(
        restored.slug, "renamed-already",
        "restore lands back on the address it was retired under, not a -2 relocation"
    );
}
