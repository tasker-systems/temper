//! The model of `estate.toml`: the scope rules that say what is in an estate, and one grain per
//! table that says how an erasure reaches the table's rows (spec D3 *Contents*, D7.4).
//!
//! Every place the manifest names a mechanism, it says whether that mechanism exists
//! ([`Mechanism::Bound`]) or is still to be built ([`Mechanism::Gap`]), so nothing reads as covered
//! by being absent.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A mechanism the manifest relies on: in TOML `{ bound = "…" }` or `{ gap = "…" }`, exactly one.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mechanism {
    /// It exists: the live name of the function, view or column that does it.
    Bound(String),
    /// It does not exist yet: `<slice>: <what>`, naming where it will be built.
    Gap(String),
}

/// `estate.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Estate {
    /// The rules that put containers in an estate, one per D3 bullet.
    pub scope: Vec<ScopeRule>,
    /// Table name → its grain. One grain per table, since a TOML table key is unique.
    pub tables: BTreeMap<String, TableGrain>,
}

/// One scope rule: a set of containers the estate holds, and what selects it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeRule {
    /// The set, in words (for example "the personal team's contexts").
    pub rule: String,
    /// What selects the set.
    pub mechanism: Mechanism,
}

/// One table's entry: its grain, and every other path that also reaches its rows.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableGrain {
    /// How an erasure reaches the table's rows.
    pub grain: Grain,
    /// Paths other than the grain's that also reach rows of this table (an edge is reached by its
    /// own home and by its endpoints), each with what follows it.
    #[serde(default)]
    pub also_reached: Vec<Reach>,
    /// Free text for the reader; empty when absent.
    #[serde(default)]
    pub note: String,
}

/// D3's grains. In TOML an inline table tagged by `kind`: `per-item`, `covered`, `identity` or
/// `declared-out`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Grain {
    /// Erased item by item, each by its own unit.
    PerItem {
        /// Whether items of other tables are homed in it (a context, a cogmap), or it is a leaf.
        container: bool,
        /// What fences an item once its erasure is declared (a flag column or a view).
        fence: Mechanism,
        /// The function that erases one item.
        unit: Mechanism,
        /// How an item names the container it is homed in. Absent for a container that is
        /// reached by the scope rules rather than by a home.
        home: Option<Home>,
    },
    /// Reached through its parent: erased by the parent's unit, unless `unit` names another.
    Covered {
        /// The path from a row to the parent that covers it.
        parent: ParentPath,
        /// What erases these rows when the parent's unit does not: absent when the parent's unit
        /// covers them.
        unit: Option<Mechanism>,
    },
    /// Erased at declaration, with the subject's identity.
    Identity {
        /// What erases it.
        mechanism: Mechanism,
    },
    /// Deliberately not erased.
    DeclaredOut {
        /// Why.
        reason: String,
        /// The retention category that keeps it, when one applies.
        retention: Option<String>,
    },
}

/// How a per-item row names its container: a CHECKed `*_table` / `*_id` pair, on `table`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Home {
    /// The table holding the pair: the item's own table, or a homes table beside it.
    pub table: String,
    /// The `*_table` column naming the container's table.
    pub table_column: String,
    /// The `*_id` column naming the container.
    pub id_column: String,
}

/// A path from a row to another table's row.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentPath {
    /// The table the path reaches.
    pub table: String,
    /// The columns that make the link: on the row's own table, or, for a path with `via`, on
    /// `table`, the table whose rows cite this one.
    pub link: Link,
    /// For a path read in reverse, from a citing table with no foreign key (`kb_remote_sources`
    /// is cited by `kb_block_provenance`'s `(source_kind, source_id)`): how the citation is read.
    /// Its presence moves `link`'s columns onto `table`.
    pub via: Option<String>,
}

/// The columns of a [`ParentPath`]: in TOML `{ fk = "…" }` or
/// `{ pair = { table_column = "…", id_column = "…" } }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Link {
    /// A foreign-key column.
    Fk(String),
    /// A CHECKed discriminator pair.
    Pair {
        /// The `*_table` column naming the other row's table.
        table_column: String,
        /// The `*_id` column naming the other row.
        id_column: String,
    },
}

/// A path that also reaches a table's rows, and what follows it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reach {
    /// The path.
    pub path: ParentPath,
    /// What follows the path to the rows.
    pub mechanism: Mechanism,
}
