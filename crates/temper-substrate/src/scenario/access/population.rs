//! Generated bulk for a measurement corpus: turns a [`PopulationDef`] distribution into real
//! resources through the **product write path**.
//!
//! Every resource here is born by firing `SeedAction::ResourceCreate`, exactly as a hand-declared
//! `AccessResourceDef` is. The projector does the rest — content blocks, chunks, the `doc_type`
//! property, and `_rebuild_resource_search_vector`. Nothing in this module writes those tables
//! directly, so a corpus cannot drift from what a real create produces.
//!
//! Two things ARE synthetic, and both are deliberate:
//!
//! * **The prose** is generated from the topic name plus deterministic filler.
//! * **The chunk vectors** are drawn around a per-topic centroid rather than embedded by ONNX.
//!   Every measurement this corpus serves depends on the distribution and cardinality of vectors,
//!   never on their semantics; 20k real bge embeddings would cost minutes and buy none of it. The
//!   vectors carry a non-model `embedded_with` marker ([`SYNTHETIC_EMBED_MARKER`]) so they can
//!   never be read as real bge output, and so the re-embed drain correctly sees them as stale.
//!
//! **Determinism is a property, not a nicety.** Task 2 compares query plans captured before and
//! after a refactor; if the corpus differed between the two captures the diff would be measuring
//! the corpus. Generation therefore uses a seeded SplitMix64 rather than the `rand` crate — the
//! same declaration produces the same corpus on any machine, and adding a dependency to get a
//! weaker guarantee would be the wrong trade.

use crate::content::{self, PreparedBlock};
use crate::events::{fire, SeedAction};
use crate::ids::{EntityId, ProfileId};
use crate::payloads;
use crate::scenario::access::model::{AccessWorld, HomeDef, PopulationDef};
use anyhow::{bail, Context, Result};
use sqlx::PgConnection;
use std::collections::HashMap;
use uuid::Uuid;

/// Stamped onto `kb_chunks.embedded_with` for every generated vector.
///
/// Deliberately NOT a model sha256: `embedded_with` means "the model that produced this vector",
/// and stamping the server's own identity onto a vector no model produced would be vouching for a
/// computation that never happened. As a non-matching value it also reads as *stale* to the
/// re-embed drain, which is the correct disposition — run the drain over this corpus and the
/// synthetic vectors are replaced by real ones.
pub const SYNTHETIC_EMBED_MARKER: &str = "synthetic:measurement-corpus";

/// Embedding width. Must match `kb_chunks.embedding vector(768)`.
const EMBED_DIM: usize = 768;

/// Filler vocabulary. Ordinary English so `to_tsvector('english', …)` stems it the way it stems
/// real prose — a filler of nonsense tokens would give the GIN index an unrepresentative term
/// distribution and make FTS selectivity measurements meaningless.
const FILLER: &[&str] = &[
    "the",
    "system",
    "records",
    "each",
    "change",
    "as",
    "an",
    "event",
    "and",
    "projects",
    "it",
    "into",
    "state",
    "which",
    "the",
    "reader",
    "observes",
    "through",
    "a",
    "gate",
    "that",
    "narrows",
    "results",
    "to",
    "what",
    "this",
    "principal",
    "may",
    "see",
    "before",
    "any",
    "ordering",
    "is",
    "applied",
    "so",
    "a",
    "page",
    "is",
    "never",
    "thinned",
    "after",
    "truncation",
    "by",
    "a",
    "filter",
    "that",
    "ran",
    "too",
    "late",
];

/// Generate every population in `world`, scaled by `scale` (1 = as declared).
///
/// Registers each generated resource under `<key_prefix>-<index>` in `resources`, so declarative
/// checks and tests can name individual members.
#[allow(clippy::too_many_arguments)]
pub async fn generate(
    tx: &mut PgConnection,
    world: &AccessWorld,
    scale: u32,
    profiles: &HashMap<String, Uuid>,
    entities: &HashMap<String, Uuid>,
    contexts: &HashMap<String, Uuid>,
    cogmaps: &HashMap<String, Uuid>,
    teams: &HashMap<String, Uuid>,
    resources: &mut HashMap<String, Uuid>,
) -> Result<()> {
    for pop in &world.populations {
        generate_one(
            tx, world, pop, scale, profiles, entities, contexts, cogmaps, teams, resources,
        )
        .await
        .with_context(|| format!("generating population '{}'", pop.key_prefix))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn generate_one(
    tx: &mut PgConnection,
    world: &AccessWorld,
    pop: &PopulationDef,
    scale: u32,
    profiles: &HashMap<String, Uuid>,
    entities: &HashMap<String, Uuid>,
    contexts: &HashMap<String, Uuid>,
    cogmaps: &HashMap<String, Uuid>,
    teams: &HashMap<String, Uuid>,
    resources: &mut HashMap<String, Uuid>,
) -> Result<()> {
    if pop.owners.is_empty() {
        bail!("population '{}' declares no owners", pop.key_prefix);
    }
    if pop.homes.is_empty() {
        bail!("population '{}' declares no homes", pop.key_prefix);
    }
    if pop.topics.is_empty() {
        bail!("population '{}' declares no topics", pop.key_prefix);
    }
    // A topic supplies the FTS vocabulary by splitting its name on '-', so a name that is empty or
    // all separators yields no terms and `% terms.len()` divides by zero. Named here beside the
    // other degenerate cases rather than reaching `prose_for` as a panic.
    if let Some(bad) = pop
        .topics
        .iter()
        .find(|t| t.split('-').all(|part| part.is_empty()))
    {
        bail!(
            "population '{}' declares topic {bad:?}, which yields no terms — a topic name supplies \
             the FTS vocabulary by splitting on '-'",
            pop.key_prefix
        );
    }

    // Emitter: named, or the world's sole/first entity.
    let emitter_name = match &pop.emitter {
        Some(n) => n.clone(),
        None => world
            .entities
            .first()
            .map(|e| e.name.clone())
            .with_context(|| {
                format!(
                    "population '{}' names no emitter and world.entities is empty",
                    pop.key_prefix
                )
            })?,
    };
    let emitter = EntityId::from(*entities.get(&emitter_name).with_context(|| {
        format!(
            "population '{}' emitter '{emitter_name}' not in world.entities",
            pop.key_prefix
        )
    })?);

    // Centroids are keyed on the topic NAME, so the same topic in two populations shares a cluster.
    let centroids: Vec<Vec<f32>> = pop.topics.iter().map(|t| centroid_for(t)).collect();

    let total = pop.count.saturating_mul(scale.max(1));
    for i in 0..total {
        let idx = i as usize;
        let topic = &pop.topics[idx % pop.topics.len()];
        let centroid = &centroids[idx % pop.topics.len()];
        let owner = ProfileId::from(
            *profiles
                .get(&pop.owners[idx % pop.owners.len()])
                .with_context(|| {
                    format!(
                        "population '{}' owner '{}' not in world.profiles",
                        pop.key_prefix,
                        pop.owners[idx % pop.owners.len()]
                    )
                })?,
        );
        let home = resolve_home(
            &pop.homes[idx % pop.homes.len()],
            &pop.key_prefix,
            contexts,
            cogmaps,
        )?;
        let doc_type = if pop.doc_types.is_empty() {
            None
        } else {
            Some(pop.doc_types[idx % pop.doc_types.len()].as_str())
        };

        let key = format!("{}-{:04}", pop.key_prefix, i);
        let title = format!("{}: {}", title_case(topic), key);
        let origin_uri = format!("synthetic://measurement-corpus/{key}");

        // Prose and vectors are both derived from (topic, index) so the whole corpus is a pure
        // function of its declaration.
        let prose = prose_for(topic, pop.words_per_resource, seed_of(&[topic, &key]));
        let mut block = content::prepare_block_deferred(0, None, &prose);
        fill_synthetic_vectors(
            &mut block,
            centroid,
            pop.topic_spread,
            seed_of(&[&key, topic]),
        );

        let fired = fire(
            &mut *tx,
            SeedAction::ResourceCreate {
                title: &title,
                origin_uri: &origin_uri,
                resource_id: None,
                home,
                owner,
                originator: None,
                blocks: std::slice::from_ref(&block),
                doc_type,
                emitter,
                segmented: false,
            },
        )
        .await?;
        let rid = match fired {
            crate::events::Fired::Resource(r) => r,
            other => bail!("ResourceCreate returned {other:?}"),
        };

        // The topic label is persisted so a reader can group by cluster without knowing how the
        // generator laid the vectors out — which is what lets the separation guard assert over the
        // database rather than over the generator's own arithmetic.
        fire(
            &mut *tx,
            SeedAction::PropertyAssert {
                resource: rid,
                key: "topic",
                value: &serde_json::Value::String(topic.clone()),
                weight: 1.0,
                emitter,
            },
        )
        .await?;

        let rid_uuid = Uuid::from(rid);
        // Through the loader's single grant sink, never a second copy — see
        // `loader::insert_resource_grants` for why this is a call and not an INSERT.
        super::loader::insert_resource_grants(
            &mut *tx,
            rid_uuid,
            Uuid::from(owner),
            &pop.grants,
            &key,
            teams,
            profiles,
        )
        .await?;

        // Refuse rather than overwrite. Populations load AFTER the hand-declared resources and
        // share one map, so a collision would replace a named referent and every `check:` naming it
        // would silently resolve to a generated row instead.
        if let Some(prior) = resources.insert(key.clone(), rid_uuid) {
            bail!(
                "population '{}' generated key {key:?}, which already names resource {prior} — a \
                 generated key must never shadow a hand-declared one",
                pop.key_prefix
            );
        }
    }
    Ok(())
}

fn resolve_home(
    home: &HomeDef,
    key_prefix: &str,
    contexts: &HashMap<String, Uuid>,
    cogmaps: &HashMap<String, Uuid>,
) -> Result<payloads::AnchorRef> {
    Ok(match home {
        HomeDef::Cogmap { name } => payloads::AnchorRef {
            table: payloads::AnchorTable::Cogmaps,
            id: *cogmaps.get(name).with_context(|| {
                format!("population '{key_prefix}' homes in unknown cogmap {name}")
            })?,
        },
        HomeDef::Context { name } => payloads::AnchorRef {
            table: payloads::AnchorTable::Contexts,
            id: match name {
                Some(n) => *contexts.get(n).with_context(|| {
                    format!("population '{key_prefix}' homes in unknown context {n}")
                })?,
                None => Uuid::now_v7(),
            },
        },
    })
}

// ─── deterministic generation ────────────────────────────────────────────────

/// SplitMix64. Chosen over the `rand` crate deliberately: the guarantee this corpus needs is that
/// the *same declaration yields the same bytes on any machine and any version*, and a dependency
/// whose generator algorithm is free to change between releases is a weaker guarantee, not a
/// stronger one.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0,1).
    fn next_f64(&mut self) -> f64 {
        // 53-bit mantissa, the standard construction.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal via Box-Muller. One of the pair is discarded — negligible here, and keeping
    /// both would make the draw order depend on call parity, which is a subtle reproducibility trap.
    fn next_gaussian(&mut self) -> f64 {
        let u1 = self.next_f64().max(f64::MIN_POSITIVE);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// FNV-1a over the parts, so a seed is a pure function of the strings that name it.
fn seed_of(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// A unit vector derived from the topic name alone.
fn centroid_for(topic: &str) -> Vec<f32> {
    let mut rng = Rng(seed_of(&["centroid", topic]));
    let v: Vec<f64> = (0..EMBED_DIM).map(|_| rng.next_gaussian()).collect();
    normalize(&v)
}

/// Fill every chunk of `block` with a vector drawn around `centroid`.
///
/// **The noise is normalized to a unit vector BEFORE being scaled**, so `spread` is the ratio of
/// noise magnitude to signal magnitude and means the same thing at any dimension. Adding
/// per-dimension gaussian noise directly — the obvious implementation — silently destroys the
/// cluster: a unit vector's components are ~N(0, 1/D), about 0.036 at D=768, so per-dimension noise
/// of 0.55 is fifteen times the signal and every vector collapses into the uniform-random case
/// where 768-dimensional distances concentrate and ANN ranking is arbitrary. Measured before the
/// fix: within-topic mean cosine distance 0.9961 against cross-topic 1.0002 — a separation of
/// 0.004, i.e. none.
fn fill_synthetic_vectors(block: &mut PreparedBlock, centroid: &[f32], spread: f64, seed: u64) {
    let mut rng = Rng(seed);
    for chunk in &mut block.chunks {
        let noise: Vec<f64> = (0..EMBED_DIM).map(|_| rng.next_gaussian()).collect();
        let noise = normalize(&noise);
        let v: Vec<f64> = centroid
            .iter()
            .zip(&noise)
            .map(|(c, n)| *c as f64 + spread * *n as f64)
            .collect();
        chunk.embedding = Some(normalize(&v));
        chunk.embedded_with = Some(SYNTHETIC_EMBED_MARKER.to_string());
    }
}

/// L2-normalize, matching what the real embedder emits — cosine distance over unnormalized vectors
/// would not be comparable with the rest of the corpus.
fn normalize(v: &[f64]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    // A zero vector cannot be normalized; it is unreachable for a gaussian draw over 768 dims, and
    // falling back to an arbitrary unit vector would silently plant an outlier in the corpus.
    debug_assert!(norm > 0.0, "gaussian draw produced a zero vector");
    v.iter().map(|x| (x / norm) as f32).collect()
}

/// Prose whose leading terms are the topic's own words, so `websearch_to_tsquery('postgres')`
/// selects the `postgres-*` topics and nothing else.
fn prose_for(topic: &str, words: u32, seed: u64) -> String {
    let terms: Vec<&str> = topic.split('-').filter(|s| !s.is_empty()).collect();
    let mut rng = Rng(seed);
    let mut out: Vec<String> = Vec::with_capacity(words as usize);
    for i in 0..words {
        // Salt the topic terms through the body at a fixed cadence rather than only at the head, so
        // a chunk cut from the middle still carries them — otherwise only the first chunk of each
        // resource would be findable by the topic's own words.
        if i % 7 == 0 {
            out.push(terms[(i as usize / 7) % terms.len()].to_string());
        } else {
            out.push(FILLER[(rng.next_u64() % FILLER.len() as u64) as usize].to_string());
        }
    }
    out.join(" ")
}

fn title_case(topic: &str) -> String {
    topic
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corpus must be a pure function of its declaration — Task 2 compares plans captured
    /// before and after a refactor, and a corpus that differed between captures would make the
    /// diff measure the corpus instead of the change.
    #[test]
    fn generation_is_deterministic_across_runs() {
        let a = centroid_for("postgres-indexing");
        let b = centroid_for("postgres-indexing");
        assert_eq!(a, b);
        assert_eq!(
            prose_for("query-planning", 40, seed_of(&["x"])),
            prose_for("query-planning", 40, seed_of(&["x"]))
        );
    }

    /// Keyed on the NAME, so the same topic in two populations shares a cluster and a query can
    /// match across a visibility boundary.
    #[test]
    fn distinct_topics_get_distinct_centroids() {
        let a = centroid_for("postgres-indexing");
        let b = centroid_for("release-process");
        assert_ne!(a, b);
        let cos: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!(
            cos.abs() < 0.3,
            "independent centroids should be near-orthogonal in 768 dims, got cos={cos}"
        );
    }

    #[test]
    fn centroids_are_unit_length() {
        let c = centroid_for("embedding-models");
        let norm: f32 = c.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm was {norm}");
        assert_eq!(c.len(), EMBED_DIM);
    }

    /// The topic's own words must reach beyond the first chunk, or only a resource's opening window
    /// would be findable by them.
    #[test]
    fn topic_terms_are_distributed_through_the_body() {
        let p = prose_for("postgres-indexing", 210, 7);
        let words: Vec<&str> = p.split(' ').collect();
        let second_half = &words[words.len() / 2..];
        assert!(
            second_half.contains(&"postgres") || second_half.contains(&"indexing"),
            "topic terms vanished from the body's second half"
        );
    }
}
