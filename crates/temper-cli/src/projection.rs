//! The read-only local projection of cloud vault state.
//!
//! `temper pull <context>` materializes every resource in a context as an
//! on-disk markdown file and records a per-context staleness cursor. The
//! projection is read-only by convention: editing a projected file changes
//! nothing on the server. See
//! `internal/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use temper_client::TemperClient;
use temper_core::context_ref::{parse_context_ref, ContextOwnerRef, ContextRef};
use temper_core::types::context::ContextRowWithCounts;
use temper_core::types::resource_view::ResourceView;
use temper_workflow::types::resource::ResourceListParams;
use temper_workflow::types::ContentResponse;
use temper_workflow::vault::Vault;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// The per-context staleness cursor, written to
/// `.temper/projection/<context>.json` after every successful pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionCursor {
    /// Server's latest event id for the context at pull time. `None` when
    /// the context had no events.
    pub last_event_id: Option<Uuid>,
    /// When the projection for this context was last refreshed.
    pub pulled_at: DateTime<Utc>,
}

/// The cursor sidecar's key for a caller-supplied context string.
///
/// A decorated ref collapses to its slug half; anything else (a bare context
/// name, a UUID) is the key verbatim. So `@me/temper`, `@j-cole-taylor/temper`
/// and `temper` all name one sidecar, which is what lets `temper pull @me/temper`
/// and a `temper status` that knows only `temper` agree.
///
/// **Derived from the ref, never from a row.** An empty context has no row to
/// derive from, and a cursor is exactly what an empty context still needs — it
/// is how "pulled, and there was nothing" stays distinct from "never pulled".
fn cursor_key(context: &str) -> String {
    match parse_context_ref(context) {
        Ok(ContextRef::OwnerSlug { slug, .. }) => slug,
        _ => context.to_string(),
    }
}

/// Absolute path of a context's cursor sidecar.
///
/// Both [`read_cursor`] and [`write_cursor`] go through here, so the key can
/// only be derived once. They did diverge: the write was rekeyed to the bare
/// context name while the read still used the caller's raw string, so
/// `pull @me/temper` filed a cursor that `check_context_staleness(.., "@me/temper")`
/// could not find.
fn cursor_path(state_dir: &Path, context: &str) -> PathBuf {
    let key = cursor_key(context);
    state_dir.join("projection").join(format!("{key}.json"))
}

/// Read a context's cursor sidecar. Returns `None` when the file is absent
/// or unparseable (a corrupt sidecar is treated as "never pulled" rather
/// than a hard error — the next pull overwrites it).
pub fn read_cursor(state_dir: &Path, context: &str) -> Result<Option<ProjectionCursor>> {
    let path = cursor_path(state_dir, context);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str::<ProjectionCursor>(&content).ok())
}

/// Atomically write a context's cursor sidecar using the standard
/// temp-file-plus-rename pattern.
///
/// `cursor_path` normalizes a decorated ref down to its slug half, so the key
/// no longer carries a `/`. The temp path is still derived from the cursor
/// `path` directly (via `set_extension`) rather than by re-joining `context` as
/// a string — belt-and-braces, so that a key which somehow does contain a
/// separator cannot silently create a second level of nesting that
/// `create_dir_all(dir)` did not prepare.
pub fn write_cursor(state_dir: &Path, context: &str, cursor: &ProjectionCursor) -> Result<()> {
    let path = cursor_path(state_dir, context);
    let dir = path.parent().ok_or_else(|| {
        TemperError::Config(format!("cursor path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(dir)?;
    // Derive the temp path from `path` itself — never by re-joining the key as
    // a string, so a separator in it cannot create a nested subdirectory that
    // `create_dir_all(dir)` did not prepare.
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let content = serde_json::to_string_pretty(cursor)?;
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Outcome of a non-blocking staleness pre-flight for one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalenessOutcome {
    /// No cursor sidecar — the context was never pulled. The check made no
    /// network call; the caller stays silent.
    NotProjected,
    /// A cursor exists and matches the server's latest event. Silent.
    Fresh,
    /// A cursor exists but the server has advanced past it. The caller warns.
    Stale,
    /// The check could not complete — offline, or the context could not be
    /// resolved. Silent (a debug log is emitted at the failure site).
    Skipped,
}

/// Compare a context's cursor against the server's latest event id for that
/// context. Pure: the staleness *decision*, with no IO. The server's id is
/// recorded into the cursor at pull time, so any divergence means at least
/// one event landed since the last pull.
fn evaluate_staleness(cursor: &ProjectionCursor, server_latest: Option<Uuid>) -> StalenessOutcome {
    if server_latest == cursor.last_event_id {
        StalenessOutcome::Fresh
    } else {
        StalenessOutcome::Stale
    }
}

/// Resolve a context ref to its row in the contexts list. Returns `None`
/// when the ref cannot be parsed, the context is not found, or the API call
/// fails — callers treat any of these as "cannot answer", not as an error.
///
/// Accepts a UUID or decorated `@owner/slug` / `+team/slug` form. Bare names
/// are rejected by the parser and return `None` (consistent with the arc's
/// hard-rejection of ambiguous bare-name addressing).
///
/// **`me` is the caller's own owner segment** (`@<handle>`, from
/// [`self_owner_ref`]), and it is what makes the `@me` arm exact. Passing
/// `None` falls back to matching any `@`-sigiled owner by slug alone — which
/// is unambiguous only while every visible profile-owned context is the
/// principal's own, and that is **not** guaranteed: `temper context share`
/// widens `resources_visible_to` for a team's members, so another profile's
/// context can be listed here with an `@<their-handle>` owner (see
/// `tests/e2e/tests/context_share_e2e.rs`). Prefer passing `me`.
async fn resolve_context_row(
    client: &TemperClient,
    context: &str,
    me: Option<&str>,
) -> Option<ContextRowWithCounts> {
    let r = parse_context_ref(context).ok()?;
    let rows = client.contexts().list().await.ok()?;
    match r {
        ContextRef::Id(id) => rows.into_iter().find(|c| Uuid::from(c.id) == id),
        ContextRef::OwnerSlug { owner, slug } => rows.into_iter().find(|c| {
            c.slug == slug
                && match &owner {
                    ContextOwnerRef::Me => match me {
                        Some(mine) => c.owner_ref == mine,
                        None => c.owner_ref.starts_with('@'),
                    },
                    ContextOwnerRef::Handle(h) => c.owner_ref == format!("@{h}"),
                    ContextOwnerRef::Team(t) => c.owner_ref == format!("+{t}"),
                }
        }),
    }
}

/// Non-blocking staleness pre-flight for one context. Reads the context's
/// cursor sidecar; only if one exists does it resolve the context id and
/// fetch the server's latest event id. Never errors and never blocks:
///
/// - no cursor             -> `NotProjected` (zero network calls)
/// - cursor + server even  -> `Fresh`
/// - cursor + server ahead -> `Stale`
/// - any failure           -> `Skipped` (debug log)
pub async fn check_context_staleness(
    client: &TemperClient,
    state_dir: &Path,
    context: &str,
) -> StalenessOutcome {
    let cursor = match read_cursor(state_dir, context) {
        Ok(Some(cursor)) => cursor,
        Ok(None) => return StalenessOutcome::NotProjected,
        Err(e) => {
            tracing::debug!("staleness check skipped: cursor read failed for {context}: {e}");
            return StalenessOutcome::Skipped;
        }
    };
    // `me` is deliberately not resolved here: this runs in the warmup pre-flight,
    // and a `GET /api/profile` round-trip to sharpen the `@me` arm is not worth
    // paying on every orientation. The residual is the loose `@me` match named on
    // [`resolve_context_row`] — a staleness verdict for a same-slug context shared
    // in from another profile. Wrong verdict, never a wrong write.
    let Some(context_id) = resolve_context_row(client, context, None)
        .await
        .map(|c| Uuid::from(c.id))
    else {
        tracing::debug!("staleness check skipped: could not resolve context '{context}'");
        return StalenessOutcome::Skipped;
    };
    let server_latest = match client.events().latest_for_context(context_id).await {
        Ok(latest) => latest,
        Err(e) => {
            tracing::debug!("staleness check skipped: latest_for_context failed: {e}");
            return StalenessOutcome::Skipped;
        }
    };
    evaluate_staleness(&cursor, server_latest)
}

/// Remove projection `.md` files for resources no longer present in the
/// context. `keep` is the set of absolute file paths the current pull
/// wrote. Walks `<vault_root>/<owner>/<context_name>/<doc_type>/*.md` across
/// every owner directory. Only `.md` files are considered; other files
/// and other contexts are never touched. Returns the number of files removed.
///
/// `context` must be the **on-disk directory name** (the context's slug/name,
/// e.g. `"temper"`), not a decorated ref like `@me/temper`. Callers should
/// derive it from the listed rows' `context_name` field rather than forwarding
/// the raw command-line ref.
///
/// **`owners` bounds the sweep, and it is load-bearing.** This walked *every*
/// owner directory once, and `keep` only ever holds the paths written for the
/// one context being pulled — so with `@me/temper` and `+acme/temper` both
/// projected, pulling either deleted the other outright. A context name is not
/// unique across owners; `temper`, `notes` and `planning` are exactly the names
/// two owners both pick.
///
/// It cannot narrow to a single directory either, because one context legitimately
/// has two spellings on disk: `@<handle>` from before identity was resolvable, and
/// `@me` after. Pruning only the current one would leave the pre-rename tree
/// standing forever. So the caller passes the set of segments *this* context can
/// be under — see `owner_candidates` — and nothing outside it is touched.
pub fn prune_context(
    vault_root: &Path,
    owners: &[String],
    context: &str,
    keep: &HashSet<PathBuf>,
) -> Result<usize> {
    let mut removed = 0usize;
    let owner_iter = match std::fs::read_dir(vault_root) {
        Ok(iter) => iter,
        // An absent vault root means there is nothing to prune. Any other IO
        // failure (permissions, etc.) is a real error and must surface.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    for owner_entry in owner_iter.flatten() {
        if !owner_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        // Skip hidden dirs such as `.temper`.
        if owner_entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let owner_name = owner_entry.file_name();
        if !owners.iter().any(|o| o.as_str() == owner_name) {
            continue;
        }
        let context_dir = owner_entry.path().join(context);
        if !context_dir.is_dir() {
            continue;
        }
        for doctype_entry in std::fs::read_dir(&context_dir)?.flatten() {
            if !doctype_entry
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            for file_entry in std::fs::read_dir(doctype_entry.path())?.flatten() {
                let path = file_entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                if !keep.contains(&path) {
                    std::fs::remove_file(&path)?;
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

/// Bound on the slug half of a projection filename, in bytes.
///
/// A single path component is capped at 255 bytes on ext4, APFS and NTFS, and
/// `sluggify` is unbounded — long agent-authored titles have produced names the
/// OS refuses to create. 120 + `-` + a 36-byte uuid + `.md` is 160 bytes, which
/// leaves the cap unreachable no matter how the title grows while keeping
/// enough of the title to recognize the file in `ls`.
///
/// **The usable budget is 238, not 255.** `Frontmatter::write_to` writes through
/// a sibling temp file named `.{filename}.frontmatter.tmp`
/// (`temper-workflow/src/frontmatter/document.rs:390`), which is 17 bytes longer
/// than the file it becomes — so the temp path hits the limit first, and the
/// error names a path that does not exist afterwards. Any future raise of this
/// constant must clear `238 - 1 - 36 - 3 = 198`.
pub const PROJECTION_SLUG_MAX_BYTES: usize = 120;

/// The owner directory segment for a resource's projection file.
///
/// **Sigiled, always.** `Vault::parse_rel` rejects an owner segment without a
/// leading `@` or `+`, so the bare handle is not a legal vault path component —
/// and `ResourceView.owner_handle` is exactly that: `p.handle` straight off the
/// readback (`temper-substrate/src/readback/mod.rs`), e.g. `j-cole-taylor`. The
/// writer used it anyway and produced a tree its own layout module could not
/// parse.
///
/// `context_owner_ref` is sigiled *by construction* — `@<handle>` for a profile
/// home, `+<team-slug>` for a team one — so it is the field to key on. The
/// fallbacks sigil the handle rather than passing it through, so no branch can
/// emit an unsigiled segment.
///
/// **`me` is what turns the caller's own handle into `@me`** — the self-relative
/// segment the layout was designed around
/// (`internal/superpowers/specs/2026-06-25-ws6-rehome-temper-next-to-public-design.md`,
/// "F6 `@me` projection dir"). Answering *is this mine?* needs the authenticated
/// profile, which the CLI holds no copy of locally — `~/.config/temper/auth.json`
/// stores a token and a device id and its `profile_id` is structurally null under
/// Auth0 — so it is resolved from the server by [`self_owner_ref`] and threaded
/// in. `None` means "identity unknown", and every branch then answers exactly as
/// it did before: `@<handle>`, correct-and-stable, one `pull` away from the
/// rename — `prune_context` walks every owner directory, so the same pull that
/// writes `@me/<ctx>/…` removes the files under `@<handle>/<ctx>/…`. It removes
/// files only: the emptied directories are left standing, and nothing collects
/// them.
fn projection_owner(row: &ResourceView, me: Option<&str>) -> String {
    let owner = owner_segment(row);
    match me {
        Some(mine) if owner == mine => SELF_OWNER_SEGMENT.to_string(),
        _ => owner,
    }
}

/// The self-relative owner directory: the caller's own resources live here.
pub const SELF_OWNER_SEGMENT: &str = "@me";

/// Every owner directory one context may occupy, given who is asking.
///
/// A context has one owner but can have two *spellings* on disk, because the
/// self-relative rewrite arrived after the tree did: files written before
/// identity was resolvable sit under `@<handle>`, and files written after sit
/// under `@me`. Both are the same context, so a prune must reach both — that is
/// what makes the rename self-cleaning rather than a duplicate tree.
///
/// **`@me` is only ever a candidate for the caller's own context.** On disk
/// `@me` means *this machine's user*, so offering it while pulling someone
/// else's context would aim the prune at the caller's own identically-named
/// tree. When identity is unknown the answer is the server's spelling alone:
/// a pre-existing `@me` tree is then left standing, which is the end that loses
/// no work.
///
/// **`@me` can also be a real handle, and then the namespace is ambiguous.** Nothing reserves
/// it: `generate_profile_handle` sluggifies the display name, so the profile called "Me" gets
/// `handle = "me"` and its `context_owner_ref` is the literal string `@me`. On disk that is the
/// same directory the caller's own contexts occupy, so pulling *their* context with the caller's
/// tree underneath it would sweep the caller's files with a `keep` set that never mentioned them —
/// the exact destruction the bound above exists to stop, re-entering through the name. When the
/// segment is `@me` and the caller is not that profile (identity unknown included), this answers
/// with **no** candidates: a stale file, never a deletion in a directory whose owner is ambiguous.
///
/// Contrast [`removal_owner_candidates`], which is deliberately less careful —
/// and may be, because it is bounded by a filename rather than a directory. A literal `@me` owner
/// is safe there for the same reason: the stem carries the resource's uuid.
fn owner_candidates(server_owner_ref: &str, me: Option<&str>) -> Vec<String> {
    if server_owner_ref == SELF_OWNER_SEGMENT && me != Some(SELF_OWNER_SEGMENT) {
        return Vec::new();
    }
    let mut owners = vec![server_owner_ref.to_string()];
    if me == Some(server_owner_ref) && server_owner_ref != SELF_OWNER_SEGMENT {
        owners.push(SELF_OWNER_SEGMENT.to_string());
    }
    owners
}

/// Every owner directory **one resource's file** may sit in.
///
/// This offers `@me` for any profile-owned context without asking who the caller
/// is, where [`owner_candidates`] refuses to — and the difference is what each
/// one is bounded by. A prune matches a *directory* and deletes everything
/// unlisted inside it, so a wrong owner costs someone else's whole context. A
/// removal matches one exact filename, and the stem carries the resource's uuid
/// (`projection_stem`), so `@me/<ctx>/<type>/<title>-<uuid>.md` can only ever be
/// *this* resource — projected under `@me` only if it was the caller's own.
///
/// That is what lets removal cover the case identity resolution cannot: the
/// writer and the remover each resolve `me` over the network and can disagree,
/// so the remover must reach the spelling the writer used even when its own
/// profile call just failed. `@me` is skipped for a team context, where the
/// writer could never have produced it.
fn removal_owner_candidates(server_owner_ref: &str) -> Vec<String> {
    let mut owners = vec![server_owner_ref.to_string()];
    if server_owner_ref.starts_with('@') && server_owner_ref != SELF_OWNER_SEGMENT {
        owners.push(SELF_OWNER_SEGMENT.to_string());
    }
    owners
}

/// The row's owner segment as the *server* names it — `@<handle>` or
/// `+<team-slug>`, never self-relative. Split out from [`projection_owner`] so
/// the self-relative rewrite is one comparison against one derivation.
fn owner_segment(row: &ResourceView) -> String {
    if let Some(owner_ref) = row.context_owner_ref.as_deref() {
        if !owner_ref.is_empty() {
            return owner_ref.to_string();
        }
    }
    if row.owner_handle.is_empty() {
        // A sparse row with neither field: the self-relative segment is the only
        // honest guess, and it is at least sigiled.
        return SELF_OWNER_SEGMENT.to_string();
    }
    format!("@{}", row.owner_handle)
}

/// The caller's own owner segment (`@<handle>`), resolved from the server.
///
/// The projection's owner directory is self-relative for the caller's own
/// contexts, and *whose* they are is a fact only the server holds: the stored
/// credential carries no usable profile id (`JwtClaims.profile_id` is populated
/// only when the JWT `sub` parses as a UUID, and an Auth0 `sub` never does), so
/// identity is resolved rather than read. `GET /api/profile` returns
/// `Profile.slug`, which **is** `kb_profiles.handle` — `profile_service.rs`
/// selects `handle AS "slug!"` and `kb_profiles` has no `slug` column — and
/// `context_owner_ref` is built as `'@' || handle`, so the two are comparable
/// verbatim.
///
/// `None` on any failure, and every caller degrades to the server's own
/// spelling rather than erroring: an unreachable profile endpoint must not turn
/// a best-effort projection tail into a failed command.
pub async fn self_owner_ref(client: &TemperClient) -> Option<String> {
    match crate::actions::runtime::ensure_profile(client).await {
        Ok(profile) => Some(format!("@{}", profile.slug)),
        Err(e) => {
            tracing::debug!("projection owner falls back to the server's spelling: {e}");
            None
        }
    }
}

/// The filename stem for a resource's projection file: the resource's decorated
/// ref with the slug half bounded.
///
/// The stem is deliberately the *same shape* as the ref printed by every
/// `list`/`show` row, so a filename can be pasted straight into
/// `temper resource show`. Identity lives in the trailing uuid — which is what
/// resolution reads and what the delete path matches on — so the readable half
/// is free to be cut.
fn projection_stem(row: &ResourceView) -> String {
    temper_workflow::operations::decorated_ref_bounded(
        &row.title,
        row.id,
        PROJECTION_SLUG_MAX_BYTES,
    )
}

/// Assemble and write a resource's projection file from an already-fetched
/// row and content. The pure-write half of [`write_resource_file`] — it
/// makes no network call.
///
/// Every caller reaches it through [`write_resource_file`] (which fetches
/// first): `pull_context`, and the create/update tails. `temper resource show`
/// used to call it directly, holding both halves already — that is the read
/// that no longer writes.
///
/// Frontmatter assembly reuses `actions::ingest::build_frontmatter_from_resource`
/// so projected files are byte-identical to sync-pulled ones. Returns the
/// absolute path written, or `None` when the resource is cogmap-homed and
/// therefore skipped (see below).
pub fn write_resource_file_from_parts(
    vault_root: &Path,
    row: &ResourceView,
    content: &ContentResponse,
    me: Option<&str>,
) -> Result<Option<PathBuf>> {
    use crate::actions::ingest;

    // A cogmap-homed resource has no context path on disk; the local vault
    // projection layout for cogmap homes is a later beat. Skip projection —
    // the cloud stays authoritative; the local cache simply doesn't
    // materialize it. (Surface B follow-up.)
    let Some(context) = row.context_name.as_deref() else {
        tracing::debug!(
            resource = %Uuid::from(row.id),
            "projection skipped: cogmap-homed resources are not projected locally yet"
        );
        return Ok(None);
    };

    let owner = projection_owner(row, me);
    let doc_type = row.doc_type_name.as_str();

    let stem = projection_stem(row);

    // Propagate a serialization failure rather than writing `null` into the projected file's
    // frontmatter — a silent `unwrap_or(Null)` here would corrupt the on-disk managed meta.
    let managed_value = content
        .managed_meta
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| TemperError::Config(format!("projection serialize managed_meta: {e}")))?;

    let fm = ingest::build_frontmatter_from_resource(ingest::BuildFrontmatterParams {
        resource: row,
        context,
        doc_type,
        canonical_owner: &owner,
        body: ingest::normalize_body_for_vault(&content.markdown),
        managed_meta: managed_value.as_ref(),
        open_meta: content.open_meta.as_ref(),
    })?;

    let path = Vault::new(vault_root).doc_file(&owner, context, doc_type, &stem);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    fm.write_to(&path)
        .map_err(|e| TemperError::Config(format!("projection write {}: {e}", path.display())))?;
    Ok(Some(path))
}

/// Fetch a resource's content and write it as a complete markdown file at
/// its canonical vault path. Returns the absolute path written.
///
/// `row` is a resource summary already obtained from a `list` call; this
/// makes one further API call (`content`) for the body + frontmatter meta,
/// then delegates the assembly + write to [`write_resource_file_from_parts`].
/// Returns `None` when the resource is cogmap-homed (projection skipped).
pub async fn write_resource_file(
    client: &TemperClient,
    vault_root: &Path,
    row: &ResourceView,
    me: Option<&str>,
) -> Result<Option<PathBuf>> {
    let content = client
        .resources()
        .content(Uuid::from(row.id))
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    write_resource_file_from_parts(vault_root, row, &content, me)
}

/// Remove a resource's projection file given a server [`ResourceView`].
///
/// A by-row convenience over [`remove_resource_file`] for the id-addressed
/// `temper resource delete` path. **Every path component is derived by the same
/// functions the writer uses** — `owner_segment` and `projection_stem` — so
/// the remover cannot look somewhere the writer never wrote.
///
/// **It takes no identity, deliberately.** The writer's owner segment depends on
/// who the caller is, but the remover does not have to re-answer that question:
/// it sweeps every spelling the file could be under
/// (`removal_owner_candidates`), which is sound because the stem carries the
/// resource's uuid. Asking again would mean a second network call that can fail
/// independently of the first, and a delete that quietly missed whenever the two
/// answers disagreed.
///
/// It could before. The owner segment came from `config.owner_for_context()`,
/// which reads `Config::subscriptions` — a field hardcoded to `Vec::new()`, so
/// it always answered `"@me"` while the writer used the bare `owner_handle`.
/// Delete's cache cleanup was therefore a silent no-op on every machine: the
/// removal targeted a path that never existed, and an absent file is a
/// deliberate success here, so nothing reported it.
pub fn remove_resource_file_for_row(vault_root: &Path, row: &ResourceView) -> Result<()> {
    // A cogmap-homed resource was never projected to disk (no context path),
    // so there is nothing to remove. Skip — same bounded edge as the writer.
    let Some(context) = row.context_name.as_deref() else {
        tracing::debug!(
            resource = %Uuid::from(row.id),
            "projection removal skipped: cogmap-homed resources are not projected locally yet"
        );
        return Ok(());
    };

    let stem = projection_stem(row);
    // Try every spelling this resource's context can be under, not just the one
    // *this* invocation resolves to. The writer and the remover each resolve
    // identity over the network, and they can disagree: a pull that reached
    // `GET /api/profile` writes `@me/…`, and a later delete whose profile call
    // fails would look under `@<handle>/…`, find nothing, and report success
    // over a surviving file. An absent file is already a silent success here, so
    // sweeping both costs nothing and removes the disagreement window.
    for owner in removal_owner_candidates(&owner_segment(row)) {
        remove_resource_file(vault_root, &owner, context, &row.doc_type_name, &stem)?;
    }
    Ok(())
}

/// Remove a resource's projection file at its canonical vault path.
///
/// A best-effort counterpart to [`write_resource_file_from_parts`], used
/// by `temper resource delete` after a successful server-side delete. An
/// already-absent file is a silent success — the projection is
/// derivative, so "the file is gone" is the desired end state either way.
pub fn remove_resource_file(
    vault_root: &Path,
    owner: &str,
    context: &str,
    doc_type: &str,
    slug: &str,
) -> Result<()> {
    let path = Vault::new(vault_root).doc_file(owner, context, doc_type, slug);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TemperError::Config(format!(
            "projection remove {}: {e}",
            path.display()
        ))),
    }
}

/// Outcome of a `pull_context` call, for the command's output line.
#[derive(Debug, Clone)]
pub struct PullSummary {
    pub context: String,
    pub written: usize,
    pub pruned: usize,
}

/// Page size for listing a context's resources. Contexts are small (tens to
/// low hundreds of resources); this paginates defensively regardless of the
/// server's own list cap.
const PULL_PAGE_SIZE: i64 = 200;

/// Materialize a whole context's resources into the local projection:
/// list every resource, write each file, prune files for resources no
/// longer present, then record the per-context staleness cursor.
///
/// Idempotent — re-running produces the same tree.
pub async fn pull_context(
    client: &TemperClient,
    config: &Config,
    context: &str,
) -> Result<PullSummary> {
    // One identity resolution per pull, threaded into every path derivation —
    // the writer's, the remover's and the prune directory's — so the tree can
    // only be keyed one way.
    let me = self_owner_ref(client).await;
    let rows = list_context_resources(client, context).await?;
    let keep = write_projection_files(client, &config.vault_root, &rows, me.as_deref()).await?;
    let pruned = prune_absent_files(
        client,
        &config.vault_root,
        context,
        &rows,
        &keep,
        me.as_deref(),
    )
    .await?;
    record_context_cursor(client, &config.state_dir, context, &rows).await?;

    Ok(PullSummary {
        context: context.to_string(),
        written: keep.len(),
        pruned,
    })
}

/// List every resource in `context`, following the server's pagination.
async fn list_context_resources(client: &TemperClient, context: &str) -> Result<Vec<ResourceView>> {
    let mut rows: Vec<ResourceView> = Vec::new();
    let mut offset: i64 = 0;
    loop {
        let params = ResourceListParams {
            context_ref: Some(context.to_string()),
            limit: Some(PULL_PAGE_SIZE),
            offset: Some(offset),
            ..Default::default()
        };
        let resp = client
            .resources()
            .list(&params)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        let page_len = resp.rows.len() as i64;
        rows.extend(resp.rows);
        if page_len < PULL_PAGE_SIZE {
            break;
        }
        offset += PULL_PAGE_SIZE;
    }
    Ok(rows)
}

/// Write each listed resource's projection file, returning the set of paths
/// that must be kept (used to drive pruning).
async fn write_projection_files(
    client: &TemperClient,
    vault_root: &Path,
    rows: &[ResourceView],
    me: Option<&str>,
) -> Result<HashSet<PathBuf>> {
    let mut keep: HashSet<PathBuf> = HashSet::new();
    for row in rows {
        if let Some(path) = write_resource_file(client, vault_root, row, me).await? {
            keep.insert(path);
        }
    }
    Ok(keep)
}

/// The **projection directory** name for a context: its `context_name`
/// (e.g. `"temper"`), never the raw ref the caller typed (`"@me/temper"`).
///
/// This is the directory half only. The staleness cursor is keyed separately by
/// [`cursor_key`], off the ref rather than off a row, because an empty context
/// has no row and still needs a cursor.
///
/// **Both branches answer with the same field.** The row branch reads
/// `context_name` — what the writer used to build the directory — and the
/// no-rows branch fetches `kb_contexts.name` for the same context, rather than
/// substituting the *slug* half of the ref. Substituting was a live defect: a
/// context named `"Temper KB"` has slug `"temper-kb"`, so emptying it made the
/// next pull prune `<vault>/@owner/temper-kb/` while the real files sat under
/// `<vault>/@owner/Temper KB/` — the stale tree survived and looked live.
///
/// The fetch is why this is async, and it is paid **only when the context has
/// no rows**: with rows, the answer is already in hand. `None` (unparseable ref,
/// unknown context, unreachable server) prunes nothing, which is the safe end —
/// a stale file, never a deletion in a directory we could not name.
async fn context_dir_name(
    client: &TemperClient,
    context: &str,
    rows: &[ResourceView],
    me: Option<&str>,
) -> Option<(Vec<String>, String)> {
    if let Some(found) = rows_dir_name(rows, me) {
        return Some(found);
    }
    resolve_context_row(client, context, me)
        .await
        .map(|c| (owner_candidates(&c.owner_ref, me), c.name))
}

/// The row branch of [`context_dir_name`]: the owner segments and `context_name`
/// off the first listed row. Split out so the no-server half stays a pure
/// function.
///
/// The owner comes from [`owner_segment`], not [`projection_owner`] — the
/// candidates are built from the **server's** spelling, and `owner_candidates`
/// is the one place that decides whether `@me` joins it.
fn rows_dir_name(rows: &[ResourceView], me: Option<&str>) -> Option<(Vec<String>, String)> {
    let row = rows.first()?;
    let name = row.context_name.clone()?;
    Some((owner_candidates(&owner_segment(row), me), name))
}

/// Prune projection files for resources no longer present in the context.
async fn prune_absent_files(
    client: &TemperClient,
    vault_root: &Path,
    context: &str,
    rows: &[ResourceView],
    keep: &HashSet<PathBuf>,
    me: Option<&str>,
) -> Result<usize> {
    match context_dir_name(client, context, rows, me).await {
        Some((owners, name)) => prune_context(vault_root, &owners, &name, keep),
        None => Ok(0),
    }
}

/// Record the per-context staleness cursor.
///
/// Keyed by the **ref the caller passed**, normalized by `cursor_key` inside
/// `write_cursor` — the same normalization `read_cursor` applies, so the two
/// ends cannot disagree however the context was spelled. Deriving it from a row
/// instead would silently skip the write for an empty context addressed by
/// UUID, which is the one case where "pulled, and there was nothing" most needs
/// to stay distinct from "never pulled".
///
/// The context's UUID comes from any listed row; an empty context yields no
/// event id, and that is a recorded `None`, not an absent cursor.
async fn record_context_cursor(
    client: &TemperClient,
    state_dir: &Path,
    context: &str,
    rows: &[ResourceView],
) -> Result<()> {
    let context_id = rows.first().and_then(|r| r.kb_context_id.map(Uuid::from));
    let last_event_id = match context_id {
        Some(cid) => client
            .events()
            .latest_for_context(cid)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?,
        None => None,
    };
    write_cursor(
        state_dir,
        context,
        &ProjectionCursor {
            last_event_id,
            pulled_at: Utc::now(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_cursor_round_trips() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        let json = serde_json::to_string(&cursor).unwrap();
        let back: ProjectionCursor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.last_event_id, cursor.last_event_id);
        assert_eq!(back.pulled_at, cursor.pulled_at);
    }

    #[test]
    fn cursor_write_then_read_round_trips() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_dir = dir.path().join(".temper");
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        write_cursor(&state_dir, "myctx", &cursor).unwrap();
        let back = read_cursor(&state_dir, "myctx").unwrap();
        assert!(back.is_some());
        assert_eq!(back.unwrap().last_event_id, cursor.last_event_id);
    }

    #[test]
    fn read_cursor_returns_none_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_dir = dir.path().join(".temper");
        assert!(read_cursor(&state_dir, "never-pulled").unwrap().is_none());
    }

    #[test]
    fn prune_removes_stale_md_keeps_listed_and_other_contexts() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        let task_dir = root.join("@me/myctx/task");
        std::fs::create_dir_all(&task_dir).unwrap();
        let keep = task_dir.join("keep.md");
        let stale = task_dir.join("stale.md");
        let notes = task_dir.join("notes.txt");
        std::fs::write(&keep, "keep").unwrap();
        std::fs::write(&stale, "stale").unwrap();
        std::fs::write(&notes, "notes").unwrap();

        let other_ctx = root.join("@me/otherctx/task");
        std::fs::create_dir_all(&other_ctx).unwrap();
        let other = other_ctx.join("other.md");
        std::fs::write(&other, "other").unwrap();

        let mut keep_set = HashSet::new();
        keep_set.insert(keep.clone());

        let pruned = prune_context(root, &["@me".to_string()], "myctx", &keep_set).unwrap();

        assert_eq!(pruned, 1, "exactly one stale .md removed");
        assert!(keep.exists(), "listed file kept");
        assert!(!stale.exists(), "unlisted .md removed");
        assert!(notes.exists(), "non-.md file untouched");
        assert!(other.exists(), "other context untouched");
    }

    /// A context name is not unique across owners, and the prune must not act as
    /// though it were.
    ///
    /// `keep` only ever holds the paths written for the **one** context being
    /// pulled, so a sweep that visited every owner directory deleted every other
    /// owner's identically-named context outright — and `temper`, `notes` and
    /// `planning` are precisely the names two owners both choose. Pulling my own
    /// `temper` destroyed the team's.
    #[test]
    fn pruning_one_owners_context_leaves_another_owners_namesake_alone() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let mine = root.join("@me/temper/task");
        let theirs = root.join("+acme/temper/task");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&theirs).unwrap();
        let my_file = mine.join("a.md");
        let their_file = theirs.join("b.md");
        std::fs::write(&my_file, "mine").unwrap();
        std::fs::write(&their_file, "theirs").unwrap();

        // A pull of my `temper`: `keep` holds only what it wrote.
        let mut keep = HashSet::new();
        keep.insert(my_file.clone());
        let pruned =
            prune_context(root, &owner_candidates("@j", Some("@j")), "temper", &keep).unwrap();

        assert_eq!(pruned, 0, "nothing of mine was stale");
        assert!(my_file.exists(), "my own listed file kept");
        assert!(
            their_file.exists(),
            "another owner's identically-named context was swept, at {}",
            their_file.display()
        );
    }

    /// The other half of the same bound: both spellings of **my own** context are
    /// reachable, so the `@<handle>` → `@me` rename cleans up after itself.
    ///
    /// This is why the prune cannot simply narrow to one directory. A tree written
    /// before identity was resolvable sits under `@<handle>`; the pull that starts
    /// writing `@me` must remove it, or the vault carries two copies forever.
    ///
    /// **Both directories hold something stale, deliberately.** An earlier draft
    /// put the kept file under `@me` and the only stale one under `@<handle>` —
    /// and passed with `@me` absent from the candidate set, because nothing it
    /// asserted ever required reaching that directory. A bite probe caught it. The
    /// name claimed a reach the assertions did not exercise.
    #[test]
    fn a_pull_prunes_both_spellings_of_my_own_context() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let old_tree = root.join("@j-cole-taylor/temper/task");
        let new_tree = root.join("@me/temper/task");
        std::fs::create_dir_all(&old_tree).unwrap();
        std::fs::create_dir_all(&new_tree).unwrap();
        // The pre-rename copy, and a resource deleted server-side since the last
        // pull — one stale file in each spelling, so reaching only one is visible.
        let pre_rename = old_tree.join("a.md");
        let deleted_upstream = new_tree.join("gone.md");
        let current = new_tree.join("a.md");
        std::fs::write(&pre_rename, "old").unwrap();
        std::fs::write(&deleted_upstream, "gone").unwrap();
        std::fs::write(&current, "new").unwrap();

        let mut keep = HashSet::new();
        keep.insert(current.clone());
        let owners = owner_candidates("@j-cole-taylor", Some("@j-cole-taylor"));
        let pruned = prune_context(root, &owners, "temper", &keep).unwrap();

        assert_eq!(pruned, 2, "one stale file in each spelling of my context");
        assert!(current.exists(), "the current file is kept");
        assert!(
            !pre_rename.exists(),
            "the pre-rename tree is left standing at {}",
            pre_rename.display()
        );
        assert!(
            !deleted_upstream.exists(),
            "the current spelling was not swept at {}",
            deleted_upstream.display()
        );
    }

    /// `@me` is offered only for the caller's OWN context.
    ///
    /// On disk `@me` names this machine's user, so offering it while pulling
    /// someone else's context would aim the prune at the caller's own
    /// identically-named tree. Identity-unknown answers with the server's
    /// spelling alone — a stale `@me` tree survives, which loses no work.
    #[test]
    fn the_self_relative_segment_is_a_candidate_only_for_my_own_context() {
        assert_eq!(
            owner_candidates("@j-cole-taylor", Some("@j-cole-taylor")),
            vec!["@j-cole-taylor".to_string(), "@me".to_string()]
        );
        assert_eq!(
            owner_candidates("@some-other-human", Some("@j-cole-taylor")),
            vec!["@some-other-human".to_string()]
        );
        assert_eq!(
            owner_candidates("+platform-eng", Some("@j-cole-taylor")),
            vec!["+platform-eng".to_string()]
        );
        assert_eq!(
            owner_candidates("@j-cole-taylor", None),
            vec!["@j-cole-taylor".to_string()]
        );
    }

    /// The writer and the remover each resolve identity over the network, and a
    /// delete must still find the file when they DISAGREE.
    ///
    /// `what_the_writer_wrote_is_what_the_remover_removes` runs both identity
    /// states, but only ever in agreement — so it could not see this. A pull that
    /// reached `GET /api/profile` writes `@me/…`; a later delete whose profile call
    /// fails resolves `None`, looks under `@<handle>/…`, finds nothing, and reports
    /// success over a file that is still there. An absent file is a silent success
    /// by design, which is exactly what makes the miss invisible.
    #[test]
    fn the_remover_finds_the_file_when_identity_resolution_disagrees() {
        for wrote_with in [Some("@j-cole-taylor"), None] {
            let dir = tempfile::TempDir::new().unwrap();
            let mut row = row_titled("A Projected Resource", Uuid::now_v7());
            row.owner_handle = "j-cole-taylor".to_string();
            row.context_owner_ref = Some("@j-cole-taylor".to_string());

            let written = write_resource_file_from_parts(
                dir.path(),
                &row,
                &body_only("# body\n"),
                wrote_with,
            )
            .unwrap()
            .unwrap();
            assert!(written.exists());

            remove_resource_file_for_row(dir.path(), &row).unwrap();

            assert!(
                !written.exists(),
                "written with me={wrote_with:?}: the file survived at {}",
                written.display()
            );
        }
    }

    /// `@me` is a sigil in the layout AND a possible handle, and the prune must not
    /// confuse the two.
    ///
    /// Nothing reserves the handle: `generate_profile_handle` sluggifies the display
    /// name, so the profile called "Me" is `@me`. Their context and the caller's own
    /// then occupy one directory. Pulling theirs with a `keep` set that names only
    /// their files would delete the caller's — the same cross-owner destruction
    /// `pruning_one_owners_context_leaves_another_owners_namesake_alone` covers,
    /// arriving through the namespace instead of through the name.
    #[test]
    fn a_literal_me_handle_never_licenses_sweeping_the_self_relative_tree() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let shared_dir = root.join("@me/temper/task");
        std::fs::create_dir_all(&shared_dir).unwrap();
        let my_own_file = shared_dir.join("mine.md");
        std::fs::write(&my_own_file, "my own work").unwrap();

        // Pulling the OTHER profile's context: nothing of mine is in `keep`.
        for me in [Some("@j-cole-taylor"), None] {
            let owners = owner_candidates(SELF_OWNER_SEGMENT, me);
            assert!(
                owners.is_empty(),
                "an ambiguous `@me` owner must offer no prune candidate (me={me:?})"
            );
            let pruned = prune_context(root, &owners, "temper", &HashSet::new()).unwrap();
            assert_eq!(pruned, 0, "nothing swept (me={me:?})");
            assert!(
                my_own_file.exists(),
                "the caller's own tree survived (me={me:?})"
            );
        }

        // But the profile that IS `@me` still prunes its own tree normally.
        let owners = owner_candidates(SELF_OWNER_SEGMENT, Some(SELF_OWNER_SEGMENT));
        assert_eq!(owners, vec!["@me".to_string()]);
        let pruned = prune_context(root, &owners, "temper", &HashSet::new()).unwrap();
        assert_eq!(pruned, 1, "its own stale file is still swept");
    }

    #[test]
    fn evaluate_staleness_equal_ids_is_fresh() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::nil())),
            StalenessOutcome::Fresh
        );
    }

    #[test]
    fn evaluate_staleness_differing_ids_is_stale() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::from_u128(1))),
            StalenessOutcome::Stale
        );
    }

    #[test]
    fn evaluate_staleness_both_none_is_fresh() {
        let cursor = ProjectionCursor {
            last_event_id: None,
            pulled_at: Utc::now(),
        };
        assert_eq!(evaluate_staleness(&cursor, None), StalenessOutcome::Fresh);
    }

    #[test]
    fn evaluate_staleness_server_advanced_from_none_is_stale() {
        let cursor = ProjectionCursor {
            last_event_id: None,
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::nil())),
            StalenessOutcome::Stale
        );
    }

    #[test]
    fn prune_returns_zero_when_vault_root_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let pruned =
            prune_context(&missing, &["@me".to_string()], "anyctx", &HashSet::new()).unwrap();
        assert_eq!(pruned, 0, "absent vault root prunes nothing, no error");
    }

    #[test]
    fn remove_resource_file_deletes_the_canonical_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let task_dir = root.join("@me/myctx/task");
        std::fs::create_dir_all(&task_dir).unwrap();
        let file = task_dir.join("doomed.md");
        std::fs::write(&file, "body").unwrap();

        remove_resource_file(root, "@me", "myctx", "task", "doomed").unwrap();

        assert!(!file.exists(), "projection file removed");
    }

    /// A row carrying only the fields the projection stem reads. Every other
    /// field is defaulted — this witnesses the filename, not the frontmatter.
    fn row_titled(title: &str, id: Uuid) -> ResourceView {
        use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
        ResourceView {
            id: ResourceId(id),
            r#ref: String::new(),
            title: title.to_string(),
            origin_uri: String::new(),
            kb_context_id: Some(ContextId(Uuid::nil())),
            context_name: Some("myctx".to_string()),
            context_slug: Some("myctx".to_string()),
            context_owner_ref: Some("@me".to_string()),
            context_ref: Some("@me/myctx".to_string()),
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "research".to_string(),
            owner_handle: "@me".to_string(),
            owner_profile_id: ProfileId(Uuid::nil()),
            originator_profile_id: ProfileId(Uuid::nil()),
            is_active: true,
            created: Utc::now(),
            updated: Utc::now(),
            body_hash: None,
            ingest_state: None,
            body_storage: None,
            managed_meta: Default::default(),
            open_meta: None,
            content: None,
        }
    }

    /// A `ContentResponse` carrying only a body — the projection writer reads
    /// `markdown` plus the two meta tiers, and the tiers are not what these
    /// filename tests are about.
    fn body_only(markdown: &str) -> ContentResponse {
        ContentResponse {
            resource_id: temper_core::types::ids::ResourceId(Uuid::nil()),
            markdown: markdown.to_string(),
            managed_meta: None,
            open_meta: None,
        }
    }

    /// The property: a projection filename is writable on this filesystem, no
    /// matter how long the title is.
    ///
    /// Agent-authored titles in enterprise rollouts have run past the point
    /// where `sluggify(title).md` exceeds the 255-byte cap a single path
    /// component gets on ext4/APFS/NTFS, and the writer then failed with
    /// `ENAMETOOLONG`. Asserting the *byte length* rather than just "the write
    /// succeeded" is deliberate: a filesystem with a looser cap (or none) would
    /// let an unbounded name through and the test would pass while the bug
    /// remained live for everyone else.
    #[test]
    fn a_projection_filename_stays_within_the_filesystem_component_limit() {
        /// The POSIX `NAME_MAX` that ext4, APFS and NTFS all land on, less the 17
        /// bytes `Frontmatter::write_to`'s `.{name}.frontmatter.tmp` sidecar adds —
        /// the temp path is the component that hits the limit first.
        const NAME_MAX: usize = 255 - 17;

        let dir = tempfile::TempDir::new().unwrap();
        // ~999 slug bytes — comfortably past the cap on its own.
        let title = "an extremely long agent authored resource title ".repeat(21);
        let id = Uuid::now_v7();

        let path = write_resource_file_from_parts(
            dir.path(),
            &row_titled(&title, id),
            &body_only("# body\n"),
            None,
        )
        .expect("an over-long title must still project")
        .expect("a context-homed resource projects to a path");

        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.len() <= NAME_MAX,
            "projection filename is {} bytes, over the {NAME_MAX}-byte component limit: {name}",
            name.len()
        );
        assert!(path.exists(), "the file was actually created at {name}");
    }

    /// The truncated half is decoration; the uuid is what identifies the file.
    /// Two resources whose titles agree for the first 120 slug bytes must not
    /// collide — under the old `sluggify(title).md` scheme they would have,
    /// because truncation without a discriminator is a collision generator.
    #[test]
    fn two_long_titles_sharing_a_prefix_project_to_distinct_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = "the same opening clause repeated until well past the bound ".repeat(4);
        let content = body_only("# body\n");

        let a = write_resource_file_from_parts(
            dir.path(),
            &row_titled(&format!("{shared} and then one ending"), Uuid::now_v7()),
            &content,
            None,
        )
        .unwrap()
        .unwrap();
        let b = write_resource_file_from_parts(
            dir.path(),
            &row_titled(&format!("{shared} and then another"), Uuid::now_v7()),
            &content,
            None,
        )
        .unwrap()
        .unwrap();

        assert_ne!(a, b, "distinct resources must not share a projection file");
        assert!(a.exists() && b.exists(), "both files survive");
    }

    /// The stem is paste-able: the filename a reader sees in `ls` resolves to
    /// the resource it holds, via the same trailing-UUID-only `parse_ref` that
    /// every printed ref uses.
    #[test]
    fn a_projection_filename_resolves_back_to_its_resource() {
        let id = Uuid::now_v7();
        let stem = projection_stem(&row_titled(&"a long title ".repeat(30), id));
        assert_eq!(
            temper_workflow::operations::parse_ref(&stem).unwrap(),
            temper_core::types::ids::ResourceId(id)
        );
    }

    /// The pairing that was broken: whatever the writer wrote, the remover
    /// must remove. Both derive every path component from the same row through
    /// the same two functions, so this asserts the property end to end rather
    /// than asserting each side's spelling.
    ///
    /// It failed before because the remover took the owner segment from
    /// `config.owner_for_context()` — always `"@me"` — while the writer used the
    /// bare `owner_handle`. `remove_resource_file` treats an absent file as
    /// success, so `temper resource delete` reported ok and left the file.
    #[test]
    fn what_the_writer_wrote_is_what_the_remover_removes() {
        // A row whose server-side owner segment is NOT "@me" — the case the old
        // remover missed. Run it at **both** identity states: unresolved (the
        // file lands under `@j-cole-taylor`) and resolved-as-mine (it lands under
        // `@me`). Either way the remover must find it, which is the property that
        // an identity threaded to one of the two would break.
        for me in [None, Some("@j-cole-taylor")] {
            let dir = tempfile::TempDir::new().unwrap();
            let mut row = row_titled("A Projected Resource", Uuid::now_v7());
            row.owner_handle = "j-cole-taylor".to_string();
            row.context_owner_ref = Some("@j-cole-taylor".to_string());

            let written =
                write_resource_file_from_parts(dir.path(), &row, &body_only("# body\n"), me)
                    .unwrap()
                    .unwrap();
            assert!(written.exists(), "writer produced {}", written.display());

            remove_resource_file_for_row(dir.path(), &row).unwrap();

            assert!(
                !written.exists(),
                "remover must delete the file the writer wrote, at {} (me={me:?})",
                written.display()
            );
        }
    }

    /// The owner segment must be a legal vault path component. `Vault::parse_rel`
    /// rejects one without an `@`/`+` sigil, and `owner_handle` is the bare
    /// handle off `p.handle` — so passing it through produced a tree the layout
    /// module could not parse.
    #[test]
    fn the_owner_segment_is_always_sigiled() {
        let id = Uuid::now_v7();

        // Sigiled context ref wins.
        let mut team = row_titled("T", id);
        team.context_owner_ref = Some("+platform-eng".to_string());
        assert_eq!(projection_owner(&team, None), "+platform-eng");

        // No context ref: the bare handle is sigiled, never passed through.
        let mut bare = row_titled("B", id);
        bare.context_owner_ref = None;
        bare.owner_handle = "j-cole-taylor".to_string();
        assert_eq!(projection_owner(&bare, None), "@j-cole-taylor");

        // A sparse row still yields something sigiled.
        let mut sparse = row_titled("S", id);
        sparse.context_owner_ref = None;
        sparse.owner_handle = String::new();
        assert_eq!(projection_owner(&sparse, None), "@me");

        // And every branch round-trips through the layout parser, at both
        // identity states.
        for row in [&team, &bare, &sparse] {
            for me in [None, Some("@j-cole-taylor")] {
                let owner = projection_owner(row, me);
                let rel = Vault::new(Path::new("/x")).rel_path(&owner, "ctx", "task", "stem");
                assert!(
                    Vault::parse_rel(&rel).is_some(),
                    "owner segment {owner:?} is not a parseable vault path component"
                );
            }
        }
    }

    /// The property F6 asked for: the caller's **own** contexts project under the
    /// self-relative `@me`, and nobody else's do.
    ///
    /// It could not hold before because the CLI had no way to answer *is this
    /// mine?* — the stored credential's `profile_id` is structurally null under
    /// Auth0, so `projection_owner` emitted `@<handle>` for everyone. Identity now
    /// arrives from `GET /api/profile` ([`self_owner_ref`]); `None` is still a
    /// legitimate answer (offline, or an unreachable profile endpoint) and must
    /// keep the old spelling rather than guess.
    #[test]
    fn my_own_context_projects_under_the_self_relative_segment() {
        let id = Uuid::now_v7();
        let me = Some("@j-cole-taylor");

        let mut mine = row_titled("M", id);
        mine.context_owner_ref = Some("@j-cole-taylor".to_string());
        assert_eq!(projection_owner(&mine, me), "@me");

        // Someone else's context, visible to me because it is shared into a team
        // I belong to, stays under their handle.
        let mut theirs = row_titled("O", id);
        theirs.context_owner_ref = Some("@some-other-human".to_string());
        assert_eq!(projection_owner(&theirs, me), "@some-other-human");

        // A team context is never self-relative — `@me` names a profile.
        let mut team = row_titled("T", id);
        team.context_owner_ref = Some("+platform-eng".to_string());
        assert_eq!(projection_owner(&team, me), "+platform-eng");

        // Identity unknown: every row keeps the server's own spelling.
        assert_eq!(projection_owner(&mine, None), "@j-cole-taylor");
    }

    /// However a context is spelled, the cursor lands in one place — so a
    /// `pull @me/temper` is found by a `temper status` that knows only `temper`.
    /// That is the whole point of the sidecar: it used to be written under the
    /// ref verbatim and read back by bare name, and `status` reported
    /// `not-projected` for a context it had just materialized.
    #[test]
    fn every_spelling_of_a_context_keys_one_cursor() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_dir = dir.path().join(".temper");
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };

        write_cursor(&state_dir, "@me/temper", &cursor).unwrap();

        for spelling in [
            "@me/temper",
            "@j-cole-taylor/temper",
            "+a-team/temper",
            "temper",
        ] {
            assert!(
                read_cursor(&state_dir, spelling).unwrap().is_some(),
                "a cursor written as `@me/temper` must be readable as `{spelling}`"
            );
        }
        // And exactly one sidecar exists — no `@me/` subdirectory beside it.
        let files: Vec<_> = std::fs::read_dir(state_dir.join("projection"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, vec!["temper.json".to_string()]);
    }

    /// A UUID ref is not a decorated ref, so it keys verbatim — and it still
    /// keys *something*. Deriving the cursor key from a row instead skipped the
    /// write entirely for an empty context addressed by UUID, collapsing
    /// "pulled, and there was nothing" into "never pulled".
    #[test]
    fn a_uuid_ref_keys_a_cursor_of_its_own() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_dir = dir.path().join(".temper");
        let id = "019fbb77-72a3-72e1-bbbd-13eb6aa64982";
        write_cursor(
            &state_dir,
            id,
            &ProjectionCursor {
                last_event_id: None,
                pulled_at: Utc::now(),
            },
        )
        .unwrap();
        assert!(read_cursor(&state_dir, id).unwrap().is_some());
    }

    /// The directory name is a separate derivation from the cursor key, and
    /// deliberately so: it comes off `context_name`, because that is what the
    /// writer used to build the path.
    ///
    /// This covers the row branch only — it is the half that needs no server. The
    /// no-rows branch fetches `kb_contexts.name`, and the thing worth witnessing
    /// there is that an emptied context prunes the directory the writer actually
    /// wrote to when name and slug differ, which needs a real context to differ:
    /// `tests/e2e/tests/projection_pull_test.rs::
    /// pull_prunes_an_emptied_context_whose_name_and_slug_differ`.
    #[test]
    fn the_projection_directory_name_comes_off_the_row() {
        let row = row_titled("Any", Uuid::now_v7());
        // `row_titled` homes the row in context "myctx".
        let mut row = row;
        row.context_owner_ref = Some("@j-cole-taylor".to_string());
        let (owners, name) = rows_dir_name(std::slice::from_ref(&row), Some("@j-cole-taylor"))
            .expect("a listed row names one");
        assert_eq!(name, "myctx");
        // The row is the caller's own, so both spellings of it are in play.
        assert_eq!(
            owners,
            vec!["@j-cole-taylor".to_string(), "@me".to_string()]
        );
        assert!(rows_dir_name(&[], None).is_none());
    }

    #[test]
    fn remove_resource_file_is_ok_when_file_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        // Never-written file: removal is a silent no-op, not an error.
        remove_resource_file(dir.path(), "@me", "myctx", "task", "ghost").unwrap();
    }
}
