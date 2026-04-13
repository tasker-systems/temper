//! `KNOWN_OPEN_FIELDS` registry + alias lookups.
//!
//! The registry is the single source of truth for which open-meta
//! field names Temper recognizes, what their canonical form is, which
//! hyphen-form aliases map to them, what value type they hold, and
//! whether they contribute edges (relationships) or are Obsidian
//! metadata (tags/aliases/date).

/// Value-type discriminator for a known open field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenFieldType {
    /// A single string scalar (e.g. `parent`, `date`).
    String,
    /// A list of strings (e.g. `relates_to`, `depends_on`).
    StringList,
    /// A list of strings with Obsidian tag semantics — NOT resource refs.
    Tags,
}

/// Category driving edge extraction policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCategory {
    /// Drives edge extraction in `edge_service`.
    Relationship,
    /// Obsidian-compatible universal, non-relational.
    Metadata,
}

/// One entry in the known-open-field registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownOpenField {
    /// Canonical key as stored in normalized `Frontmatter` value.
    pub canonical: &'static str,
    /// Alternate keys accepted at parse time, all normalized to `canonical`.
    pub aliases: &'static [&'static str],
    /// Shape of the value.
    pub field_type: OpenFieldType,
    /// Whether the field drives edges or is metadata.
    pub category: FieldCategory,
}

/// The authoritative list of open fields Temper knows about. Order is
/// load-bearing: `canonical::serialize` uses registry order to group
/// relationships before metadata in emitted YAML.
pub const KNOWN_OPEN_FIELDS: &[KnownOpenField] = &[
    // Relationships — drive edges in `edge_service`.
    KnownOpenField {
        canonical: "relates_to",
        aliases: &["relates-to"],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "depends_on",
        aliases: &["depends-on"],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "extends",
        aliases: &[],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "references",
        aliases: &[],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "preceded_by",
        aliases: &["preceded-by"],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "derived_from",
        aliases: &["derived-from"],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "parent",
        aliases: &[],
        field_type: OpenFieldType::String,
        category: FieldCategory::Relationship,
    },
    // Metadata — Obsidian-compatible, NOT resource refs.
    KnownOpenField {
        canonical: "tags",
        aliases: &[],
        field_type: OpenFieldType::Tags,
        category: FieldCategory::Metadata,
    },
    KnownOpenField {
        canonical: "aliases",
        aliases: &[],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Metadata,
    },
    KnownOpenField {
        canonical: "date",
        aliases: &[],
        field_type: OpenFieldType::String,
        category: FieldCategory::Metadata,
    },
];

/// Look up a known open field by either its canonical name or one of
/// its aliases. Returns `None` for unknown keys.
pub fn lookup(key: &str) -> Option<&'static KnownOpenField> {
    KNOWN_OPEN_FIELDS
        .iter()
        .find(|f| f.canonical == key || f.aliases.contains(&key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_ten_entries() {
        assert_eq!(KNOWN_OPEN_FIELDS.len(), 10);
    }

    #[test]
    fn every_canonical_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for f in KNOWN_OPEN_FIELDS {
            assert!(
                seen.insert(f.canonical),
                "duplicate canonical: {}",
                f.canonical
            );
        }
    }

    #[test]
    fn lookup_by_canonical_resolves_each_entry() {
        for f in KNOWN_OPEN_FIELDS {
            let found = lookup(f.canonical).expect("canonical hits");
            assert_eq!(found.canonical, f.canonical);
        }
    }

    #[test]
    fn lookup_by_hyphen_alias_resolves_to_canonical() {
        let cases = [
            ("relates-to", "relates_to"),
            ("depends-on", "depends_on"),
            ("preceded-by", "preceded_by"),
            ("derived-from", "derived_from"),
        ];
        for (alias, expected) in cases {
            let found = lookup(alias).expect("alias hits");
            assert_eq!(found.canonical, expected);
        }
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("not_a_field").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn relationships_use_registry_order() {
        let rels: Vec<&'static str> = KNOWN_OPEN_FIELDS
            .iter()
            .filter(|f| matches!(f.category, FieldCategory::Relationship))
            .map(|f| f.canonical)
            .collect();
        assert_eq!(
            rels,
            vec![
                "relates_to",
                "depends_on",
                "extends",
                "references",
                "preceded_by",
                "derived_from",
                "parent",
            ]
        );
    }

    #[test]
    fn metadata_uses_registry_order() {
        let meta: Vec<&'static str> = KNOWN_OPEN_FIELDS
            .iter()
            .filter(|f| matches!(f.category, FieldCategory::Metadata))
            .map(|f| f.canonical)
            .collect();
        assert_eq!(meta, vec!["tags", "aliases", "date"]);
    }

    #[test]
    fn tags_is_metadata_not_relationship() {
        let tags = lookup("tags").expect("tags exists");
        assert!(matches!(tags.category, FieldCategory::Metadata));
        assert!(matches!(tags.field_type, OpenFieldType::Tags));
    }
}
