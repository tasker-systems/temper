//! Two vocabularies, one wire form — and this test is what keeps the second true of the first.
//!
//! `temper_substrate::payloads` describes events in general (and is what physically writes
//! `kb_events."references"`); `temper_core::types::admin` describes **admin-ledger** entries,
//! which are a narrower thing: bounded by event type, firewalled from cognition by their NULL
//! anchor, and read only through a per-act-family gate. They are separate types **on purpose** —
//! the distinction is an intent boundary the compiler enforces, so ledger data cannot drift into
//! cognition paths (or the reverse) without someone writing a conversion and thereby declaring it.
//! See the note in `temper_core::types::admin`; **do not "simplify" this by merging them.**
//!
//! What the separation must not buy is divergence in the wire form, because both sides read and
//! write the same column. So: same bytes, different meaning, proven rather than assumed.
//!
//! This crate is where the test can live at all — it is the one place both crates are in scope.
//!
//! **What this actually guarantees, stated precisely** — an earlier version of this comment claimed
//! "add a variant on either side and this stops compiling", which overclaims in two ways:
//!
//! 0. **One substrate rel has no mirror, on purpose.** `RefRel::Erasure` is stored but withheld from
//!    the wire (`admin_ledger_service::reaches_the_wire`) until it can join `LedgerRefRel` without
//!    breaking clients built at the 0.6 pin. `mirror_rel` answers `None` for it, and
//!    `the_withheld_rel_does_not_decode_as_the_core_mirror` pins that the withholding is what keeps
//!    the page decodable.
//!
//! 1. **The compile-time half is one-directional.** `mirror_rel`/`mirror_kind` match exhaustively on
//!    the *substrate* enums, so a new **substrate** variant breaks the build until it is mirrored.
//!    A new **core** variant does not. That asymmetry is correct rather than a gap: the core types
//!    are only ever produced by decoding what substrate wrote, so a core-only variant is
//!    unconstructible in practice — but it is not what the old sentence promised.
//! 2. **`ALL_RELS`/`ALL_KINDS` below are hand-maintained.** Nothing derives them from the enums (no
//!    `strum` in this workspace), so a variant that is added to the exhaustive `match` but forgotten
//!    here is mirrored yet never *serialized* by any assertion. Both lists are complete today
//!    (7 rels, 11 kinds); if that stops being cheap to eyeball, derive them rather than trusting the
//!    eyeball.
//!
//! The stakes for a rename specifically: renames are per-variant `#[serde(rename)]` on both sides,
//! so divergence is one typo — and `admin_ledger_service::to_wire_page` turns an undecodable
//! reference into a whole-page `ApiError::Internal`, i.e. one bad row denies an entire audit read.
//! That is what these assertions are actually protecting.

use temper_core::types::admin::{LedgerRef, LedgerRefKind, LedgerRefRel, LedgerRefTarget};
use temper_substrate::payloads::{AnchorTable, EventRef, RefRel, RefTarget};
use uuid::Uuid;

/// Every `RefRel` variant, paired with its core mirror — or `None` for a rel withheld from the wire.
/// Written as an exhaustive `match` on purpose: adding a substrate variant makes this stop
/// compiling, which is a better failure than a list that silently omits the new one.
fn mirror_rel(r: RefRel) -> Option<LedgerRefRel> {
    match r {
        RefRel::Supersedes => Some(LedgerRefRel::Supersedes),
        RefRel::DerivedFrom => Some(LedgerRefRel::DerivedFrom),
        RefRel::Touches => Some(LedgerRefRel::Touches),
        RefRel::Subject => Some(LedgerRefRel::Subject),
        RefRel::Principal => Some(LedgerRefRel::Principal),
        RefRel::Request => Some(LedgerRefRel::Request),
        RefRel::Erasure => None,
    }
}

/// The rels that have a wire mirror — what the serialization assertions below range over.
fn mirrored_rels() -> impl Iterator<Item = (RefRel, LedgerRefRel)> {
    ALL_RELS
        .iter()
        .filter_map(|&r| mirror_rel(r).map(|m| (r, m)))
}

/// Same contract as `mirror_rel`, for the anchor vocabulary.
fn mirror_kind(k: AnchorTable) -> LedgerRefKind {
    match k {
        AnchorTable::Contexts => LedgerRefKind::Contexts,
        AnchorTable::Cogmaps => LedgerRefKind::Cogmaps,
        AnchorTable::Blobs => LedgerRefKind::Blobs,
        AnchorTable::Resources => LedgerRefKind::Resources,
        AnchorTable::Edges => LedgerRefKind::Edges,
        AnchorTable::ContentBlocks => LedgerRefKind::ContentBlocks,
        AnchorTable::Teams => LedgerRefKind::Teams,
        AnchorTable::Profiles => LedgerRefKind::Profiles,
        AnchorTable::Connections => LedgerRefKind::Connections,
        AnchorTable::MachineClients => LedgerRefKind::MachineClients,
        AnchorTable::Events => LedgerRefKind::Events,
    }
}

const ALL_RELS: &[RefRel] = &[
    RefRel::Supersedes,
    RefRel::DerivedFrom,
    RefRel::Touches,
    RefRel::Subject,
    RefRel::Principal,
    RefRel::Request,
    RefRel::Erasure,
];

const ALL_KINDS: &[AnchorTable] = &[
    AnchorTable::Contexts,
    AnchorTable::Cogmaps,
    AnchorTable::Blobs,
    AnchorTable::Resources,
    AnchorTable::Edges,
    AnchorTable::ContentBlocks,
    AnchorTable::Teams,
    AnchorTable::Profiles,
    AnchorTable::Connections,
    AnchorTable::MachineClients,
    AnchorTable::Events,
];

#[test]
fn every_rel_serializes_identically_on_both_sides() {
    for (rel, mirror) in mirrored_rels() {
        let substrate = serde_json::to_value(rel).expect("substrate rel");
        let core = serde_json::to_value(mirror).expect("core rel");
        assert_eq!(substrate, core, "rel {rel:?} differs across the mirror");
    }
}

#[test]
fn every_anchor_kind_serializes_identically_on_both_sides() {
    for &kind in ALL_KINDS {
        let substrate = serde_json::to_value(kind).expect("substrate kind");
        let core = serde_json::to_value(mirror_kind(kind)).expect("core kind");
        assert_eq!(substrate, core, "kind {kind:?} differs across the mirror");
    }
}

/// The shape the handler actually round-trips: a whole `EventRef` written by substrate must
/// deserialize into the core mirror without loss, for every combination.
#[test]
fn a_substrate_event_ref_deserializes_into_the_core_mirror() {
    let id = Uuid::now_v7();

    for (rel, mirror) in mirrored_rels() {
        for &kind in ALL_KINDS {
            let written = EventRef {
                rel,
                target: RefTarget { kind, id },
            };
            let json = serde_json::to_value(written).expect("encode");

            let read: LedgerRef =
                serde_json::from_value(json.clone()).expect("substrate ref must decode as core");

            assert_eq!(
                read,
                LedgerRef {
                    rel: mirror,
                    target: LedgerRefTarget {
                        kind: mirror_kind(kind),
                        id,
                    },
                },
                "round-trip lost meaning for {json}"
            );

            // And back the other way — the handler encodes core types into responses that the
            // client decodes, so the mirror has to be symmetric, not merely permissive.
            assert_eq!(
                serde_json::to_value(read).expect("re-encode"),
                json,
                "core re-encode diverged from the substrate wire form"
            );
        }
    }
}

/// FAILS IF the withheld rel becomes decodable by the core mirror without `mirror_rel` and
/// `admin_ledger_service::reaches_the_wire` being revisited. While it fails to decode, publishing
/// it would break the page (and any client built at the 0.6 pin), which is why it is withheld;
/// once `LedgerRefRel` gains it, this test goes red and both decisions are taken together.
#[test]
fn the_withheld_rel_does_not_decode_as_the_core_mirror() {
    for &rel in ALL_RELS.iter().filter(|&&r| mirror_rel(r).is_none()) {
        let written = EventRef {
            rel,
            target: RefTarget {
                kind: AnchorTable::Events,
                id: Uuid::now_v7(),
            },
        };
        let json = serde_json::to_value(written).expect("encode");
        assert!(
            serde_json::from_value::<LedgerRef>(json.clone()).is_err(),
            "{json} now decodes as LedgerRef: wire it through mirror_rel and reaches_the_wire"
        );
    }
}
