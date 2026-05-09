//! ResourceRef — identifier for resource-action commands.
//!
//! Slug uniqueness is scoped to (owner, context, doctype); UUID is globally
//! unique. Every resource-action command (`Show`, `Update`, `Delete`, sync
//! variants) accepts either form. The enum shape (rather than two `Option`
//! fields) makes "exactly one form populated" a compile-time guarantee.

use serde::{Deserialize, Serialize};

use crate::types::ids::ResourceId;

/// Identifies a resource for a command that targets an existing resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ResourceRef {
    /// Globally-unique reference. Resolves directly without scoping fields.
    Uuid {
        #[serde(rename = "resource_id")]
        id: ResourceId,
    },
    /// Owner-qualified slug-based reference. Maps to the canonical
    /// `kb://<owner>/<context>/<doctype>/<slug>` URI form.
    Scoped {
        owner: String,
        context: String,
        doctype: String,
        slug: String,
    },
}

impl ResourceRef {
    /// Construct a UUID-based reference.
    pub fn uuid(id: ResourceId) -> Self {
        Self::Uuid { id }
    }

    /// Construct an owner-scoped reference. Argument order matches
    /// `Vault::canonical_uri(owner, context, doc_type, ident)`.
    pub fn scoped(
        owner: impl Into<String>,
        context: impl Into<String>,
        doctype: impl Into<String>,
        slug: impl Into<String>,
    ) -> Self {
        Self::Scoped {
            owner: owner.into(),
            context: context.into(),
            doctype: doctype.into(),
            slug: slug.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn scoped_constructor_sets_fields() {
        let r = ResourceRef::scoped("@me", "temper", "task", "hello-world");
        match r {
            ResourceRef::Scoped {
                owner,
                context,
                doctype,
                slug,
            } => {
                assert_eq!(owner, "@me");
                assert_eq!(context, "temper");
                assert_eq!(doctype, "task");
                assert_eq!(slug, "hello-world");
            }
            ResourceRef::Uuid { .. } => panic!("expected Scoped variant"),
        }
    }

    #[test]
    fn uuid_constructor_sets_id() {
        let id = ResourceId(Uuid::nil());
        let r = ResourceRef::uuid(id);
        match r {
            ResourceRef::Uuid { id: got } => assert_eq!(got, id),
            ResourceRef::Scoped { .. } => panic!("expected Uuid variant"),
        }
    }

    #[test]
    fn scoped_round_trips_via_serde() {
        let r = ResourceRef::scoped("@me", "temper", "task", "foo");
        let s = serde_json::to_string(&r).unwrap();
        let back: ResourceRef = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn uuid_round_trips_via_serde() {
        let r = ResourceRef::uuid(ResourceId(Uuid::nil()));
        let s = serde_json::to_string(&r).unwrap();
        let back: ResourceRef = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn scoped_carries_team_owner() {
        let r = ResourceRef::scoped("+team-acme", "engineering", "doc", "design-spec");
        match &r {
            ResourceRef::Scoped {
                owner,
                context,
                doctype,
                slug,
            } => {
                assert_eq!(owner, "+team-acme");
                assert_eq!(context, "engineering");
                assert_eq!(doctype, "doc");
                assert_eq!(slug, "design-spec");
            }
            ResourceRef::Uuid { .. } => panic!("expected Scoped variant"),
        }

        // serde wire form must include owner
        let s = serde_json::to_string(&r).unwrap();
        assert!(
            s.contains("\"owner\":\"+team-acme\""),
            "serde body did not include owner: {s}"
        );
    }
}
