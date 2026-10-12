//! The model of `estate.toml`: the scope rules that say what is in an estate, and one grain per
//! table that says how an erasure reaches the table's rows.
//!
//! Every place the manifest names a mechanism, it says whether that mechanism exists
//! ([`Mechanism::Bound`]) or is uncovered work tracked by a task ([`Mechanism::Gap`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};
use uuid::Uuid;

/// A mechanism the manifest relies on: in TOML `{ bound = "…" }` or
/// `{ gap = { task = "<uuid>", what = "…" } }`, exactly one.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mechanism {
    /// It exists: the live name of the function, view or column that does it.
    Bound(String),
    /// It does not exist: what is uncovered, and the task that tracks it.
    Gap(Gap),
}

/// Uncovered work: what no mechanism reaches, and the task that tracks it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    /// The task, as a hyphenated lowercase UUID.
    #[serde(deserialize_with = "task_id")]
    pub task: Uuid,
    /// What is uncovered, in plain words. Never blank.
    #[serde(deserialize_with = "what")]
    pub what: String,
}

/// A task id: a UUID written in its hyphenated lowercase form, so a task is never abbreviated.
fn task_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let raw = String::deserialize(deserializer)?;
    match Uuid::try_parse(&raw) {
        Ok(id) if id.hyphenated().to_string() == raw => Ok(id),
        _ => Err(serde::de::Error::custom(format!(
            "task {raw:?} is not a UUID in hyphenated lowercase form"
        ))),
    }
}

/// A gap's `what`: trimmed, and refused when blank.
fn what<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let raw = String::deserialize(deserializer)?;
    let what = raw.trim();
    if what.is_empty() {
        return Err(serde::de::Error::custom("a gap's `what` is blank"));
    }
    Ok(what.to_owned())
}

/// `estate.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Estate {
    /// The rules that put containers in an estate.
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
    /// Free text for the reader, trimmed; empty when absent.
    #[serde(default, deserialize_with = "super::trimmed")]
    pub note: String,
}

/// How an erasure reaches a table's rows. In TOML an inline table tagged by `kind`: `per-item`,
/// `covered`, `identity` or `declared-out`.
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
        /// The retention category under which a deployment keeps what the mechanism would
        /// otherwise erase, when one applies.
        retention: Option<String>,
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
    /// The columns that make the link: on the row's own table, or, for a path with `cited_by`,
    /// on `table`, the table whose rows cite this one.
    pub link: Link,
    /// For a path read in reverse: the discriminator on `table` that says a citing row cites
    /// this table. Its presence moves `link`'s columns onto `table`.
    pub cited_by: Option<CitedBy>,
}

/// The discriminator a citing table uses to say which table a row of it cites:
/// `cited_by = { column = "source_kind", value = "remote" }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CitedBy {
    /// The discriminator column, on the citing table.
    pub column: String,
    /// The value of `column` that marks a citation of this table.
    pub value: String,
}

/// The columns of a [`ParentPath`]: in TOML `{ fk = "…" }`, `{ ref = "…" }` or
/// `{ pair = { table_column = "…", id_column = "…" } }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Link {
    /// A column holding the other row's id under a foreign-key constraint.
    Fk(String),
    /// A uuid column holding the other row's id with no foreign-key constraint
    /// (`kb_teams.personal_of`).
    Ref(String),
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
    /// The one column of the row the mechanism is for, when it is not the whole row (the personal
    /// team's `description`, which the identity grain's mechanism does not reach). Absent: the
    /// whole row.
    pub column: Option<String>,
}
