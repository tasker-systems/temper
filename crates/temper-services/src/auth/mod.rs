//! Shared authentication + authorization orchestration for both surfaces.
//!
//! The gate *sequence* lives here exactly once. temper-api and temper-mcp both
//! call these functions and map [`AuthzError`] to their own transport; neither
//! re-implements the ordering. Adding a future gate is one edit here, enforced
//! on every surface.
//!
//! **The seam owns principal construction.** A surface hands in a verified token
//! and gets back an [`AuthenticatedProfile`]; it never builds an `AuthClaims`, and
//! nothing here accepts one. That is the whole enforcement mechanism: `AuthClaims`
//! is still a public type a surface *can* construct, but with `authenticate`,
//! `classify`, `Principal` and `resolve_from_claims` all crate-private, a forged one
//! has nowhere to go. It is inert rather than forbidden.
//!
//! This closes the level below the one `normalize.rs` closed. PR #384 made
//! *classification* total, so no surface could say "unrecognized ⇒ human" — but each
//! surface still hand-built its own human `AuthClaims`, and they disagreed:
//! temper-api ran a three-rung email ladder, temper-mcp set `email: ""` and
//! auto-provisioned. Any surface that can construct an `AuthClaims` can construct a
//! `PrincipalKind::Human`, which is precisely the asymmetry that made #384's bug
//! asymmetric. One constructor, one ladder (`email.rs`), one answer per token.
//!
//! Entry points:
//! - [`authenticate_caller`] — the token path. Verified [`RawJwtClaims`] + the raw token ⇒ a
//!   classified [`Caller`], holding exactly one of [`HumanPrincipal`] / [`MachinePrincipal`].
//!   Both surfaces' every authed request.
//! - [`authenticate_caller_existing_only`] — the same token path, LOOKUP-ONLY: it refuses rather
//!   than provisioning. For flows that are not registration routes (the Slack account-link
//!   callback, the declared fallback). Same seam, same Level 1 gate, same classification, and a
//!   machine takes the same machine resolver; it differs only in never minting a profile.
//! - [`resolve_federated_human`] — the federated path. An assertion already
//!   authenticated out-of-band (SAML/HMAC, no JWT) ⇒ resolved-or-JIT'd profile.
//! - [`require_system_access`] — Level 2, consuming proof of Level 1.
//!
//! The two token paths are crate-private (`authenticate_token` and
//! `authenticate_token_existing_only`), so no surface can hold an unclassified
//! [`AuthenticatedProfile`] from a token.
//!
//! Two levels form a typestate chain:
//! 1. `authenticate` (crate-private, reached only via `authenticate_token`) — resolve
//!    the profile + `is_active`. Runs on every authed request on both surfaces.
//!    Yields [`AuthenticatedProfile`].
//! 2. [`require_system_access`] — consumes proof of Level 1, adds the access gate.
//!    Runs on the gated tier of both surfaces. Yields [`SystemAuthorized`].

use sqlx::PgPool;

use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, PrincipalKind, Profile};

mod email;
pub mod machine_door;
mod normalize;
pub mod secret;
pub use normalize::{RawJwtClaims, MACHINE_PROVIDER_TAG};
// Crate-private on purpose: classification is a decision the seam makes, not one a
// surface is shown. A surface that can see `Principal` can pattern-match its way back
// to hand-building the human arm — the exact drift this module exists to prevent.
pub(crate) use normalize::{classify, Principal};

use crate::error::{ApiError, ApiResult};
use crate::services::profile_service;
use crate::state::AppState;

/// The reason an authn/authz gate refused a request. Each surface maps these to
/// its own transport (HTTP status / rmcp error); the variants are the shared
/// vocabulary of *why*, never the words on the wire.
#[derive(Debug)]
pub enum AuthzError {
    /// The token is machine-shaped but not coherently so — classification (crate-private,
    /// see `normalize.rs`) refused it. Carries the reason (already logged by the seam) so a
    /// surface can decide how much of it to say on the wire. As of the network-door ruling 7,
    /// the API says all of it: the 401 body is `machine credential refused: {why}` — the
    /// sentences are static, operator-actionable statements about the caller's own token,
    /// and a distinct body is what lets the relay's tool layer split this arm from the
    /// expired-in-flight 401 it used to share a body with.
    Refused(&'static str),
    /// The human email ladder fell off its bottom rung: no `email` claim, no cached
    /// auth link, and `/userinfo` did not answer. Distinct from [`Self::ProfileResolution`]
    /// because nothing was resolved — we could not even name the human, so no write
    /// was attempted.
    EmailResolution(ApiError),
    /// `resolve_from_claims` failed (DB error, missing link data, etc.).
    ProfileResolution(ApiError),
    /// The `has_system_access` gate check itself failed (DB error) — distinct
    /// from a clean `SystemAccessDenied`, so surfaces can keep the pre-seam
    /// "failed to check system access" diagnostic instead of collapsing it into
    /// the resolve-failure message.
    AccessCheck(ApiError),
    /// The resolved profile's principal standing is `Deactivated` — the Level-1 kill-switch,
    /// sourced from `kb_principal_standing` since Phase 2 dropped `kb_profiles.is_active`.
    Deactivated { profile_id: uuid::Uuid },
    /// The profile lacks approved standing (`kb_principal_standing`).
    /// Carries the id so a surface can build its own denial payload, and the
    /// standing machine's typed [`temper_principal::Refusal`] for the state that
    /// failed the gate — computed once, here, so every surface renders the same
    /// refusal kind rather than re-deriving (or dropping) it.
    SystemAccessDenied {
        profile_id: uuid::Uuid,
        refusal: temper_principal::Refusal,
    },
    /// Classifying the authenticated profile as human or machine (`classify_caller`) could not
    /// read whether it is a machine. Always an internal error: the request is refused without
    /// telling the client anything about its credential, and never falls through to the human arm.
    Classification(ApiError),
}

/// **The token path.** A verified JWT ⇒ an authenticated, active profile.
///
/// The one entry point for both surfaces' authed requests. The surface verifies the
/// signature (each has its own audience) and decodes into [`RawJwtClaims`]; from
/// there the seam owns everything — classification, the human email ladder, claim
/// construction, and the Level 1 gates. `token` is the raw bearer, needed only for
/// the ladder's `/userinfo` rung.
///
/// The machine arm **must not** run the email ladder: an M2M principal has no email
/// and no `/userinfo` to ask, so a ladder on that path would be an authentication
/// failure dressed as a lookup. That ordering is load-bearing, not incidental — see
/// `machine_token_authenticates_without_running_the_email_ladder`.
pub(crate) async fn authenticate_token(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthenticatedProfile, AuthzError> {
    let claims = claims_from_token(state, raw, token).await?;

    authenticate(&state.pool, &claims, state.config.auth.mode).await
}

/// The classify + human-email-ladder block, shared by both token entry points.
///
/// Private, and it stays private: it is the *only* constructor of a human `AuthClaims` on
/// the token path, so both entry points are guaranteed to agree on classification, provider
/// name and the email ladder. A surface reaches it only by handing in a `RawJwtClaims` it
/// verified — never an `AuthClaims` it authored.
///
/// The machine arm must NOT run the email ladder: an M2M principal has no email and no
/// `/userinfo` to ask, so a ladder on that path would be an authentication failure dressed
/// as a lookup. That ordering is load-bearing — see
/// `machine_token_authenticates_without_running_the_email_ladder`.
async fn claims_from_token(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthClaims, AuthzError> {
    match classify(raw) {
        Principal::Machine(machine) => Ok(machine),
        Principal::Refuse(why) => {
            // `sub` is KEPT here, unlike the human sites: `classify` only returns `Refuse` for a
            // machine-*shaped* token (an `@clients` subject / M2M grant that is incoherent), so this
            // `sub` is a machine client identifier, not the human OAuth `sub` §5 targets. It is not
            // cross-linkable to a person and is the one identifier this rejection has to point at.
            tracing::warn!(sub = %raw.sub, why, "rejected: unclassifiable machine-shaped token");
            Err(AuthzError::Refused(why))
        }
        Principal::Human => {
            let (email, email_verified) = email::resolve_email_from_claims(state, raw, token)
                .await
                .map_err(AuthzError::EmailResolution)?;
            Ok(AuthClaims {
                principal_kind: PrincipalKind::Human,
                provider: state.config.auth_provider_name.clone(),
                external_user_id: raw.sub.clone(),
                email,
                email_verified,
                exp: raw.exp,
                iat: raw.iat,
            })
        }
    }
}

/// The refusal [`authenticate_caller_existing_only`] gives a validly signed identity that maps to
/// no temper profile. Named so a caller can tell it from a machine-credential refusal without
/// matching on prose.
pub const NO_EXISTING_PROFILE: &str = "no existing temper profile for this identity";

/// **The token path, LOOKUP-ONLY.** A verified JWT ⇒ an *existing* profile, or a refusal.
///
/// Same seam as [`authenticate_token`] — it takes raw claims and the bearer, and builds the
/// `AuthClaims` itself via `claims_from_token`, so no surface can hand it forged ones. It
/// differs in exactly one way: it never provisions.
///
/// Used by the Slack account-link callback. Connecting Slack is not a registration route: the
/// profile INSERT that auto-provisioning performs fires `trg_sync_system_membership`, which in
/// `open` mode joins EVERY auto-join team. See the T2 spec, D3.
///
/// Also the declared fallback's lookup (temper-api's `routes::fallback`): a mistyped URL is not a
/// registration route either.
///
/// Level 1's post-resolution gate is **not** skipped: this shares `gate_resolved_profile`
/// with `authenticate`, so an inactive profile is refused here exactly as it is on the login
/// path. Level 2 ([`require_system_access`]) remains the caller's to apply, identical to the
/// login path.
///
/// A machine token takes the machine resolver, exactly as on the login path. That resolver only
/// looks a registration up and never creates one, so it is lookup-only already, and a machine gets
/// the answer it gets on any bearer route (registered, unregistered, revoked, deactivated) rather
/// than the human path's machine-shape refusal.
pub(crate) async fn authenticate_token_existing_only(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthenticatedProfile, AuthzError> {
    let claims = claims_from_token(state, raw, token).await?;
    if claims.principal_kind == PrincipalKind::Machine {
        return authenticate(&state.pool, &claims, state.config.auth.mode).await;
    }

    let profile = profile_service::resolve_existing_human_from_claims(&state.pool, &claims)
        .await
        .map_err(AuthzError::ProfileResolution)?
        .ok_or_else(|| {
            // The raw OAuth `sub` is deliberately NOT emitted, even though this is the one window §5
            // flags where `sub` is the *only* identifier that exists — a valid token that maps to no
            // temper profile, so there is no `profile_id` to carry instead. §5 left the choice
            // between a salted-hash `enduser.id` and emitting nothing to the implementer; nothing is
            // chosen here. A `google-oauth2|…` value would join our exported traces to the same
            // person in unrelated systems, and this is a narrow account-link refusal, not a hot path:
            // it stays correlatable to its request through the root span's trace context, and the
            // Slack side identifies the caller for the rare triage that needs it. Decided 2026-08-01.
            tracing::info!("lookup-only authentication: refused (no existing temper profile)");
            AuthzError::Refused(NO_EXISTING_PROFILE)
        })?;

    gate_resolved_profile(&state.pool, profile, &claims).await
}

/// **The federated path.** An identity asserted by a trusted peer, not a token.
///
/// Called by the internal SAML reconcile endpoint, whose assertion the co-deployed
/// Authorization Server has *already* validated server-to-server (HMAC over the body,
/// per `require_internal_signature`) before the token is minted. There is no JWT here
/// and nothing to classify: the caller is trusted, and this only resolves-or-JITs the
/// profile the minted token will later resolve to. `provider` must be the server's
/// configured provider name — never a payload field — so both paths land on the same
/// profile.
///
/// The trust assumption is exactly "the caller authenticated the assertion". It is not
/// a hole in the machine gate: #384's machine-shape guard at the top of
/// `resolve_human_from_claims` still covers this path, so an assertion carrying an
/// `@clients`-suffixed `external_user_id` is refused here as it is everywhere else.
pub async fn resolve_federated_human(
    pool: &PgPool,
    provider: &str,
    external_user_id: &str,
    email: &str,
    email_verified: Option<bool>,
) -> ApiResult<Profile> {
    let claims = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: provider.to_string(),
        external_user_id: external_user_id.to_string(),
        email: email.to_string(),
        email_verified,
        // exp/iat are unused by resolve_from_claims; supply zero rather than inventing a clock.
        exp: 0,
        iat: 0,
    };

    // Human by construction, so the machine gate's mode never applies.
    profile_service::resolve_human_from_claims(pool, &claims).await
}

/// Level 1 — authentication. Verified+normalized claims → a resolved, active profile.
///
/// Runs on **every** authenticated request on **both** surfaces, reached only through
/// [`authenticate_token`]. Crate-private: a surface cannot hand this function claims it
/// built itself, which is what makes a forged `AuthClaims` inert (module doc).
///
/// `mode` is the instance's auth mode, which bounds the machine registrations a machine token can
/// reach (`machine_door::reachable_issuer`).
pub(crate) async fn authenticate(
    pool: &PgPool,
    claims: &AuthClaims,
    mode: crate::auth_config::AuthMode,
) -> Result<AuthenticatedProfile, AuthzError> {
    let profile = profile_service::resolve_from_claims(pool, claims, mode)
        .await
        .map_err(AuthzError::ProfileResolution)?;

    gate_resolved_profile(pool, profile, claims).await
}

/// Level 1's post-resolution tail: every gate that applies *after* a profile is resolved,
/// whichever door resolved it.
///
/// Factored out so [`authenticate`] (resolve-or-provision) and
/// [`authenticate_token_existing_only`] (lookup-only) cannot drift: a narrowing that skipped
/// a gate would be a hole, not a narrowing. It is deliberately the *whole* tail — resolution
/// is the only thing the two paths do differently, so anything after it belongs here, and a
/// future Level 1 gate added here binds both paths by construction.
///
/// Takes the profile by value and returns the [`AuthenticatedProfile`]: this is that type's **sole**
/// constructor, so passing the gate and holding proof of Level 1 are the same act.
///
/// `pub(crate)` rather than private only so `crate::test_support` can mint an honest proof — it
/// runs this gate for real rather than stepping around it, which is why a fixture seeded
/// `Deactivated` is refused there exactly as it would be in production.
pub(crate) async fn gate_resolved_profile(
    pool: &PgPool,
    profile: Profile,
    claims: &AuthClaims,
) -> Result<AuthenticatedProfile, AuthzError> {
    // Level-1 kill-switch, repointed off the dropped `kb_profiles.is_active` onto principal
    // standing (Phase 2). A `Deactivated` standing bars authentication entirely; `Denied` and
    // `Revoked` deliberately pass Level 1 so they can still reach the auth-only request/review
    // routes — only Level 2 (`require_system_access`) refuses them.
    let standing = crate::services::standing_service::load(
        pool,
        temper_core::types::ids::ProfileId::from(profile.id),
    )
    .await
    .map_err(AuthzError::AccessCheck)?;
    if standing == Some(temper_principal::Standing::Deactivated) {
        return Err(AuthzError::Deactivated {
            profile_id: profile.id,
        });
    }

    Ok(AuthenticatedProfile {
        profile,
        claims: claims.clone(),
    })
}

/// Proof that a request carries a resolved, non-deactivated identity — Level 1.
///
/// SEALED: the fields are private and `gate_resolved_profile` is this module's only constructor,
/// so a struct-literal forgery outside `temper_services::auth` is a compile error: no crate can
/// hold proof of authentication without passing the gate. It lives here, beside its gate, rather
/// than in a shared crate such as temper-core, because the seal needs the private fields and the
/// only constructor in the same module. It sits beside [`SystemAuthorized`] and [`SystemAdmin`] because all three are the same kind of
/// thing: a proof, obtainable only from the gate that establishes it.
///
/// Surfaces receive one from the seam and read it through the accessors below; they never build one.
#[derive(Debug, Clone)]
pub struct AuthenticatedProfile {
    profile: Profile,
    claims: AuthClaims,
}

impl AuthenticatedProfile {
    /// The resolved profile this proof was minted for.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// The token claims the profile was resolved from.
    pub fn claims(&self) -> &AuthClaims {
        &self.claims
    }

    /// Consume the proof for its profile. The caller owns the returned profile for THIS
    /// request — it threads down the call chain as a per-request value and is never
    /// stored or cached beyond it.
    pub fn into_profile(self) -> Profile {
        self.profile
    }
}

/// Proof that the caller is a **person**: authenticated, and classified human by `classify_caller`.
///
/// SEALED like [`AuthenticatedProfile`]: the field is private and `classify_caller` is the only
/// constructor, so holding one means the classification ran and said human. There is no conversion
/// from [`MachinePrincipal`] or from a bare [`AuthenticatedProfile`] — that absence is the guarantee,
/// and the acts a machine may not take are typed to require this proof.
///
/// `Clone` because a request extension and an extractor both hand out owned values; cloning a proof
/// yields a proof of the same kind, never of the other.
#[derive(Debug, Clone)]
pub struct HumanPrincipal(AuthenticatedProfile);

impl HumanPrincipal {
    /// The authenticated identity this proof was minted for.
    pub fn authenticated(&self) -> &AuthenticatedProfile {
        &self.0
    }

    /// The person's profile id.
    pub fn profile_id(&self) -> ProfileId {
        ProfileId::from(self.0.profile.id)
    }
}

/// Reads through to the authenticated identity. Coercion runs one way only — a person's proof can be
/// read as an unclassified profile, never the reverse — so it cannot be used to forge one.
impl std::ops::Deref for HumanPrincipal {
    type Target = AuthenticatedProfile;

    fn deref(&self) -> &AuthenticatedProfile {
        &self.0
    }
}

/// Proof that the caller is a **machine**: authenticated, and classified machine by
/// `classify_caller`. Sealed exactly as [`HumanPrincipal`] is; the two have no conversion between
/// them, so a machine can never be handed to an act that requires a person.
#[derive(Debug, Clone)]
pub struct MachinePrincipal(AuthenticatedProfile);

impl MachinePrincipal {
    /// The authenticated identity this proof was minted for.
    pub fn authenticated(&self) -> &AuthenticatedProfile {
        &self.0
    }

    /// The machine's profile id.
    pub fn profile_id(&self) -> ProfileId {
        ProfileId::from(self.0.profile.id)
    }
}

/// Reads through to the authenticated identity, one way only, as [`HumanPrincipal`]'s does.
impl std::ops::Deref for MachinePrincipal {
    type Target = AuthenticatedProfile;

    fn deref(&self) -> &AuthenticatedProfile {
        &self.0
    }
}

/// A classified caller: exactly one of the two proofs. Minted only by `classify_caller`, and since
/// each arm wraps a sealed proof, holding a `Caller` means the classification ran — the variants are
/// public so a surface can match on them, not so it can build one.
#[derive(Debug, Clone)]
pub enum Caller {
    Human(HumanPrincipal),
    Machine(MachinePrincipal),
}

/// Reads through to the authenticated identity, one way only, as the two proofs' do.
impl std::ops::Deref for Caller {
    type Target = AuthenticatedProfile;

    fn deref(&self) -> &AuthenticatedProfile {
        self.authenticated()
    }
}

impl Caller {
    /// The authenticated identity, whichever kind the caller is.
    pub fn authenticated(&self) -> &AuthenticatedProfile {
        match self {
            Self::Human(h) => h.authenticated(),
            Self::Machine(m) => m.authenticated(),
        }
    }

    /// The caller's profile id, whichever kind the caller is.
    pub fn profile_id(&self) -> ProfileId {
        ProfileId::from(self.authenticated().profile.id)
    }
}

/// **The one classification.** An authenticated profile ⇒ a [`Caller`], and the only constructor
/// of [`HumanPrincipal`] and [`MachinePrincipal`].
///
/// - Machine claims ⇒ `Machine`. No read: G3 already refused a machine token with no client row.
/// - Human claims and `is_machine_profile` (any client row, revoked or not) ⇒ `Machine`, logged. No
///   profile moves between kinds today, so this arm is unreachable — but a profile that is a machine
///   at all is never handed a person's proof on the strength of its token's shape.
/// - Human claims and no client row ⇒ `Human`.
///
/// A failed read refuses with [`AuthzError::Classification`] — a 5xx, never a 401/403 telling the
/// client its credential is bad, and never a fall-through to the human arm.
pub(crate) async fn classify_caller(
    pool: &PgPool,
    authed: AuthenticatedProfile,
) -> Result<Caller, AuthzError> {
    if authed.claims.principal_kind == PrincipalKind::Machine {
        return Ok(Caller::Machine(MachinePrincipal(authed)));
    }

    let profile_id = ProfileId::from(authed.profile.id);
    let is_machine = crate::services::machine_client_service::is_machine_profile(pool, profile_id)
        .await
        .map_err(|err| {
            tracing::error!(%profile_id, ?err, "caller classification: machine-profile read failed");
            AuthzError::Classification(ApiError::Internal(
                "failed to classify the caller".to_string(),
            ))
        })?;

    if is_machine {
        tracing::warn!(
            %profile_id,
            "caller classification: human-shaped token resolved to a machine profile; classified as machine"
        );
        return Ok(Caller::Machine(MachinePrincipal(authed)));
    }

    Ok(Caller::Human(HumanPrincipal(authed)))
}

/// `authenticate_token`, then `classify_caller`: a verified JWT ⇒ a classified [`Caller`]. The
/// entry point for a surface's authed requests, so classification lives in this module with the
/// proofs it mints.
pub async fn authenticate_caller(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<Caller, AuthzError> {
    let authed = authenticate_token(state, raw, token).await?;
    classify_caller(&state.pool, authed).await
}

/// `authenticate_token_existing_only`, then `classify_caller` — the lookup-only path, for a
/// credential act outside the request middleware (the Slack account-link callback), which must not
/// be the one door that skips classification.
pub async fn authenticate_caller_existing_only(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<Caller, AuthzError> {
    let authed = authenticate_token_existing_only(state, raw, token).await?;
    classify_caller(&state.pool, authed).await
}

/// Proof that a profile passed **both** levels: authenticated *and*
/// system-authorized. Only obtainable from [`require_system_access`], which
/// only accepts an [`AuthenticatedProfile`] — so the type makes it impossible
/// to run Level 2 without having passed Level 1.
///
/// SEALED: the field is private, so the only way to hold one is [`require_system_access`].
/// A struct-literal forgery outside this module is a compile error — the enforcement the
/// doc comment above has always *claimed* but, with a `pub` field, did not have.
#[derive(Debug)]
pub struct SystemAuthorized(AuthenticatedProfile);

impl SystemAuthorized {
    /// The authenticated identity this proof was minted for.
    pub fn authenticated(&self) -> &AuthenticatedProfile {
        &self.0
    }
}

/// Level 2 — system authorization. Consumes proof of Level 1, adds the
/// gating-team access gate. Runs on the gated tier of both surfaces.
pub async fn require_system_access(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
) -> Result<SystemAuthorized, AuthzError> {
    let has_access = crate::services::access_service::has_system_access(
        pool,
        ProfileId::from(authed.profile.id),
    )
    .await
    .map_err(AuthzError::AccessCheck)?;

    if !has_access {
        // The typed reason comes straight from the standing machine at the gate — the one
        // computation every surface inherits. `admit` returns `Err(Refusal)` for exactly the
        // state that just failed; the `Ok` arm is only reachable on a race (approved between
        // the gate check and here), in which case the generic `NoStanding` is the safe
        // fallback. Read-only: `admit` decides, it never writes.
        let refusal =
            crate::services::standing_service::admit(pool, ProfileId::from(authed.profile.id))
                .await
                .err()
                .unwrap_or(temper_principal::Refusal::NoStanding);
        return Err(AuthzError::SystemAccessDenied {
            profile_id: authed.profile.id,
            refusal,
        });
    }

    Ok(SystemAuthorized(authed.clone()))
}

/// Proof the caller is a system admin (D10 governance / spec §3). SEALED: the private field means the
/// only way to hold one is [`require_system_admin`], which checks the DB. Forging one by struct
/// literal is a compile error outside this module — so a pure-admin fn that takes `&SystemAdmin`
/// cannot be reached without the gate having run. It carries the acting admin's [`ProfileId`], the
/// only thing admin fns need for governance/standing/ledger writes.
#[derive(Debug)]
pub struct SystemAdmin(ProfileId);

impl SystemAdmin {
    /// The acting admin — recorded as `actor` on every governance/standing/ledger write.
    pub fn actor(&self) -> ProfileId {
        self.0
    }
}

/// Level 3 — governance check. A sibling of [`require_system_access`] off Level 1, not a chain on top
/// of it: it asks `is_system_admin`, which since 20261205000010 is a governance row AND live standing
/// (`has_system_access`), so a governance row that outlives the principal's standing confers nothing.
///
/// Takes a [`HumanPrincipal`], so a machine cannot be handed to it — that is a compile error, not a
/// refusal, and it is why no code path can hold a `SystemAdmin` for a machine. (The #1089 triggers
/// make `is_system_admin` false for every machine as well; the type is what makes the question
/// unaskable.) Returns a plain [`ApiError::Forbidden`] on denial — admin denial needs none of
/// `AuthzError::SystemAccessDenied`'s CLI-presentation payload.
pub async fn require_system_admin(pool: &PgPool, human: &HumanPrincipal) -> ApiResult<SystemAdmin> {
    let actor = human.profile_id();
    if crate::services::access_service::is_system_admin(pool, actor).await? {
        Ok(SystemAdmin(actor))
    } else {
        Err(ApiError::Forbidden)
    }
}

/// [`require_system_admin`] on a connection rather than the pool: the same plain read of
/// `is_system_admin`, taking no lock. This is the pre-check a door runs on a pooled connection
/// before it opens its transaction (team-role rank plan P1 step 1), and the read for a surface
/// that only answers a question (an invitation list, the admin ledger). A decision that must
/// hold against a demotion until its transaction commits goes through
/// [`require_system_admin_locked`] instead, on the transaction, under the door's locks.
///
/// This, [`require_system_admin_locked`] and [`require_system_admin`] are the only places a
/// [`SystemAdmin`] is minted in a production build.
pub async fn require_system_admin_on(
    conn: &mut sqlx::PgConnection,
    human: &HumanPrincipal,
) -> ApiResult<SystemAdmin> {
    let actor = human.profile_id();
    let is_admin = sqlx::query_scalar!("SELECT is_system_admin($1)", *actor)
        .fetch_one(&mut *conn)
        .await?
        .unwrap_or(false);
    if is_admin {
        Ok(SystemAdmin(actor))
    } else {
        Err(ApiError::Forbidden)
    }
}

/// The admin re-check of a write transaction, under its locks (team-role rank plan P1, P3). Call it
/// only on an open transaction: the lock it takes is what makes the answer hold, and on an
/// autocommit connection that lock is dropped when the statement ends, protecting nothing. A
/// door's pool pre-check is [`require_system_admin_on`], a plain read.
///
/// The caller is an admin only when `is_system_admin_locked` says so: their
/// `kb_principal_governance` row was there to lock `FOR KEY SHARE`, and the lock is held to the end
/// of the transaction, so a demotion either committed first or waits for this transaction.
pub async fn require_system_admin_locked(
    tx: &mut sqlx::PgConnection,
    human: &HumanPrincipal,
) -> ApiResult<SystemAdmin> {
    let actor = human.profile_id();
    if is_system_admin_locked(tx, actor).await? {
        Ok(SystemAdmin(actor))
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Whether `profile` is a system admin, read so the answer holds to the end of the caller's
/// transaction. It takes `profile`'s `kb_principal_governance` row `FOR KEY SHARE`. A demotion
/// deletes that row (`principal_governance_set`, and the `kb_profiles` cascade), which conflicts
/// with `KEY SHARE`, so the demotion either committed first (no row, so the answer is false) or
/// waits until this transaction ends. A grant committed after the lock found no row does not
/// count, because nothing would hold it against a demotion. Transaction only, as for
/// [`require_system_admin_locked`].
///
/// Lock order: a demotion locks nothing, around its delete, that a door holds. Before the delete it
/// takes no team row, invitation row or advisory lock: a Revoke or Deactivate locks the subject's
/// standing row, and an admin's act then locks the two governance rows, the subject's and its own,
/// in ascending profile id order (`access_service::still_admin`), so two admins demoting each
/// other never wait in a cycle; a machine credential's revocation takes its credential row after
/// that pair. After
/// the delete it only appends its events and revokes the subject's refresh tokens, which no door
/// locks. So a caller may take this lock before or after its own team, member or invitation rows
/// (the invitation re-check takes it before the invitation rows it then revokes) and never waits on
/// a demotion in a cycle. A later change that makes a demotion lock a door's rows after its delete
/// would break this.
///
/// The `is_system_admin` read after the lock is the governance row just locked AND live standing
/// (20261205000010). The standing half is a plain MVCC read and takes no row lock, so it adds no
/// lock-order site. It still holds to the end of the transaction: the only transitions out of
/// `Approved` (Revoke and Deactivate, `temper_principal::transition`) delete the governance row in
/// the same transaction, which conflicts with the `KEY SHARE` held here, so they either committed
/// first (the answer is false) or wait. A direct `UPDATE` of standing takes no such path; that is the
/// unlocked drift the read-time AND exists to refuse. It must not grow a read that takes a lock, or
/// a team row: that would make this a second lock-order site.
pub(crate) async fn is_system_admin_locked(
    tx: &mut sqlx::PgConnection,
    profile: ProfileId,
) -> ApiResult<bool> {
    Ok(is_system_admin_locked_raw(tx, profile).await?)
}

/// [`is_system_admin_locked`] with the database error as is, for a caller that classifies it (an
/// act's retry loop, which retries a deadlock).
pub(crate) async fn is_system_admin_locked_raw(
    tx: &mut sqlx::PgConnection,
    profile: ProfileId,
) -> Result<bool, sqlx::Error> {
    let locked = sqlx::query_scalar!(
        "SELECT profile_id FROM kb_principal_governance WHERE profile_id = $1 FOR KEY SHARE",
        *profile,
    )
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    Ok(locked
        && sqlx::query_scalar!("SELECT is_system_admin($1)", *profile)
            .fetch_one(&mut *tx)
            .await?
            .unwrap_or(false))
}

/// The admin re-check of a door whose authority resolved on its system-admin arm on the pool: on
/// the door's write transaction, before its first write, it asks again with
/// [`is_system_admin_locked`]. `false` means a demotion committed since the pool read (one still in
/// flight makes this wait until it commits); the door then answers with the refusal it gives a
/// caller with no authority, so a demoted admin learns nothing a non-admin does not. A decision on
/// any other arm read no governance, so there is nothing to ask again and this answers `true`
/// without a query. Transaction only, as for [`is_system_admin_locked`], and under the same lock
/// order.
pub(crate) async fn admin_arm_holds(
    tx: &mut sqlx::PgConnection,
    profile: ProfileId,
    on_admin_arm: bool,
) -> ApiResult<bool> {
    if !on_admin_arm {
        return Ok(true);
    }
    is_system_admin_locked(tx, profile).await
}

/// The bare-id spelling of [`require_system_admin`], **test-only**. It mints from a `ProfileId` with
/// no classification behind it, so it exists only where `Principal::Bare` does — the test harness —
/// and a production build cannot name it.
#[cfg(any(test, feature = "test-harness"))]
pub async fn require_system_admin_by_id(pool: &PgPool, actor: ProfileId) -> ApiResult<SystemAdmin> {
    if crate::services::access_service::is_system_admin(pool, actor).await? {
        Ok(SystemAdmin(actor))
    } else {
        Err(ApiError::Forbidden)
    }
}

/// The one sentence a machine principal is refused with on every act outside content and workflow.
/// Fixed, so an operator can find it in telemetry and a client can recognise it; it names nothing
/// about the act or the subject.
pub const MACHINE_PRINCIPAL_REFUSAL: &str = "this action is not available to a machine principal";

/// The refusal for a machine principal on an act it may not take: `403` under
/// `FORBIDDEN_DETAIL_CODE` with [`MACHINE_PRINCIPAL_REFUSAL`].
pub fn machine_principal_refused() -> ApiError {
    ApiError::ForbiddenDetail(MACHINE_PRINCIPAL_REFUSAL.to_string())
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
