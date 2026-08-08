use anyhow::Result;
use clap::Parser;
use temper_core::types::home::HomeAnchor;
use temper_substrate::{embed::embed_chunks, substrate, write::materialize};

/// `temper-substrate` harness binary over the shared substrate.
///
/// The `migrate` command used to live here. It moved to the `temper-migrate` binary, whose crate
/// depends on sqlx and nothing heavy — this one pulls ort in through `temper-ingest(embed)`, and
/// the deploy compiles the migrate binary before it applies any schema.
#[derive(Parser)]
#[command(name = "temper-substrate")]
enum Cmd {
    /// Embed content blocks then materialize a cogmap's emergent telos-lens regions (spec §1). The
    /// lens name selects the region-set over the same substrate (S6f plurality).
    Materialize {
        /// Cogmap name to materialize.
        #[arg(default_value = "onboarding-cogmap")]
        cogmap: String,
        /// Lens name (e.g. `telos-default`, `telos-default-propheavy`).
        #[arg(default_value = "telos-default")]
        lens: String,
    },
    /// Load an access-scenario fixture's topology and generated populations into `DATABASE_URL`.
    ///
    /// The point is a corpus big and uneven enough that a MEASUREMENT over it is not vacuous. The
    /// same fixture a `#[sqlx::test]` loads at declared size loads here at `--scale`× that size, so
    /// the test-sized and measurement-sized corpora are one declaration and cannot drift.
    ///
    /// Not idempotent and not a migration: it appends rows and will fail on a second run against
    /// the same database (profile handles are UNIQUE). Recreate the volume, or point it at a
    /// scratch database.
    SeedCorpus {
        /// Path to an access-scenario YAML carrying a `populations:` block.
        #[arg(
            long,
            default_value = "crates/temper-substrate/tests/fixtures/access-scenarios/measurement-corpus.yaml"
        )]
        fixture: String,
        /// Multiplier applied to every population's declared `count`.
        #[arg(long, default_value_t = 1)]
        scale: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cmd = Cmd::parse();
    let pool = substrate::connect().await?;
    match cmd {
        Cmd::Materialize { cogmap: name, lens } => {
            embed_chunks(&pool).await?;
            let cogmap = substrate::cogmap_by_name(&pool, &name).await?;
            // Materialization is attributed to the entity that seeded this cogmap (its bound steward) —
            // a real referent, not "latest event".
            let emitter = substrate::cogmap_genesis_emitter(&pool, cogmap).await?;
            let outcome = materialize(&pool, HomeAnchor::Cogmap(cogmap), &lens, emitter).await?;
            println!(
                "materialized {} region(s) for '{}' (lens '{}')\nmembership: {}",
                outcome.regions, name, lens, outcome.membership_fingerprint
            );
        }
        Cmd::SeedCorpus { fixture, scale } => seed_corpus(&pool, &fixture, scale).await?,
    }
    Ok(())
}

/// Load a fixture and then REPORT what was built, per principal.
///
/// The report is not decoration. A corpus can be large and still unable to answer the question it
/// was built for — if the gate passes ~every row, or the team arms are empty, or nothing carries an
/// embedding, every measurement over it comes back green and meaningless. `measurement_corpus.rs`
/// asserts these properties at test size; printing them here is how an operator confirms they
/// survived scaling, rather than assuming they did.
async fn seed_corpus(pool: &sqlx::PgPool, fixture: &str, scale: u32) -> Result<()> {
    use temper_substrate::scenario::access::{self, model::AccessScenario};
    use temper_substrate::scenario::bootseed;

    let doc: AccessScenario = serde_yaml::from_str(&std::fs::read_to_string(fixture)?)?;
    println!("seeding '{}' from {fixture} at scale {scale}", doc.name);

    bootseed::seed_system(pool).await?;
    let loaded = access::load_scaled(pool, &doc.world, scale).await?;

    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE is_active")
        .fetch_one(pool)
        .await?;
    let chunks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE is_current AND embedding IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    println!(
        "\n{} resources registered, {total} live, {chunks} embedded chunks",
        loaded.resources.len()
    );

    println!("\nvisible fraction per principal (the gate's discriminating power):");
    let mut handles: Vec<&String> = loaded.profiles.keys().collect();
    handles.sort();
    for handle in handles {
        let id = loaded.profiles[handle];
        let seen: i64 = sqlx::query_scalar("SELECT count(*) FROM resources_visible_to($1)")
            .bind(id)
            .fetch_one(pool)
            .await?;
        let owned: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_resource_homes WHERE owner_profile_id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await?;
        let pct = if total > 0 {
            100.0 * seen as f64 / total as f64
        } else {
            0.0
        };
        // `via team/grant` is `seen - owned`: the arms that are EMPTY on the deployment whose
        // numbers this corpus exists to replace.
        println!(
            "  {handle:<10} {seen:>7} / {total} ({pct:5.1}%)   owned {owned:>6}, via team/grant {:>6}",
            seen - owned
        );
    }
    Ok(())
}
