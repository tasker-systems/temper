//! System boot-seed loader: seeds what any temper system needs (the event-type registry + the global
//! system lenses), separately from any scenario. Idempotent. The global lenses are created via the
//! reusable `lens_create` function, attributed to a canonical `system` actor.

use crate::events::{fire, SeedAction};
use crate::ids::EntityId;
use crate::scenario::model::BootSeed;
use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

/// Path to the canonical boot-seed, resolved from the crate dir so CWD doesn't matter.
const SYSTEM_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/system.yaml"
);

/// The canonical event-type names from the system boot-seed (the ledger vocabulary), exposed so the
/// migration's synthesis bootstrap can seed the registry by name without the full [`seed_system`]
/// (which also writes the system actor + global lenses and needs a substrate pool). One
/// source of truth for the vocabulary — `tests/fixtures/seeds/system.yaml`.
pub fn system_event_type_names() -> Result<Vec<String>> {
    let boot: BootSeed = serde_yaml::from_str(&std::fs::read_to_string(SYSTEM_SEED)?)?;
    Ok(boot.event_types)
}

pub async fn seed_system(pool: &PgPool) -> Result<()> {
    let boot: BootSeed = serde_yaml::from_str(&std::fs::read_to_string(SYSTEM_SEED)?)?;

    // The canonical system actor (events require a NOT NULL emitter). handle is UNIQUE; entity name is not.
    let profile: Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_profiles (handle, display_name, system_access) VALUES ('system','System','admin') \
         ON CONFLICT (handle) DO UPDATE SET display_name=EXCLUDED.display_name RETURNING id"
    )
    .fetch_one(pool)
    .await?;
    let emitter: Uuid = match sqlx::query_scalar!(
        "SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'",
        profile
    )
    .fetch_optional(pool)
    .await?
    {
        Some(id) => id,
        None => sqlx::query_scalar!(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,'system','{}'::jsonb) RETURNING id",
            profile
        )
        .fetch_one(pool)
        .await?,
    };

    // Registry rows + their published contract: stamp payload_schema/schema_version from the
    // committed tests/fixtures/payloads/<name>.v1.schema.json snapshots (payload spec §6 — repo,
    // registry, and Rust types are one chain; the snapshot test pins repo==types, this pins
    // registry==repo). A name with no snapshot (foreign/not-yet-typed) stays NULL = unregistered/
    // permissive.
    let payloads_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/payloads");
    for et in &boot.event_types {
        let schema: Option<serde_json::Value> =
            std::fs::read_to_string(format!("{payloads_dir}/{et}.v1.schema.json"))
                .ok()
                .map(|s| serde_json::from_str(&s))
                .transpose()?;
        // `category` rides the same insert. Migrations 20260718000020 / 20260719000010 stamp it by
        // name, but `reset_schema` TRUNCATEs the registry and lets this loop rebuild it — so without
        // carrying the classification here, `grant_created`/`grant_revoked` would come back as
        // 'domain' and the trail's belt-and-braces filter would silently classify them wrong on that
        // baseline. Single source: `payloads::ADMIN_EVENT_NAMES` / `SYSTEM_EVENT_NAMES`.
        let category = if crate::payloads::ADMIN_EVENT_NAMES.contains(&et.as_str()) {
            "admin"
        } else if crate::payloads::SYSTEM_EVENT_NAMES.contains(&et.as_str()) {
            "system"
        } else {
            "domain"
        };
        sqlx::query!(
            "INSERT INTO kb_event_types (name, payload_schema, schema_version, category) \
             VALUES ($1, $2, 1, $3) \
             ON CONFLICT (name) DO UPDATE SET payload_schema = EXCLUDED.payload_schema, \
                                              schema_version = EXCLUDED.schema_version, \
                                              category = EXCLUDED.category",
            et,
            schema as Option<serde_json::Value>,
            category,
        )
        .execute(pool)
        .await?;
    }

    // Global system lenses (cogmap_id NULL), idempotent on (NULL, name).
    for l in &boot.lenses {
        let exists: Option<Uuid> = sqlx::query_scalar!(
            "SELECT id FROM kb_cogmap_lenses WHERE cogmap_id IS NULL AND name=$1",
            l.name
        )
        .fetch_optional(pool)
        .await?;
        if exists.is_none() {
            let mut tx = pool.begin().await?;
            fire(
                &mut tx,
                SeedAction::LensCreate {
                    cogmap: None,
                    lens: l,
                    emitter: EntityId::from(emitter),
                },
            )
            .await?;
            tx.commit().await?;
        }
    }
    Ok(())
}
