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
    /// Slug-based reference scoped by doctype + context.
    Scoped {
        slug: String,
        doctype: String,
        context: String,
    },
}

impl ResourceRef {
    /// Construct a UUID-based reference.
    pub fn uuid(id: ResourceId) -> Self {
        Self::Uuid { id }
    }

    /// Construct a scoped (slug-based) reference.
    pub fn scoped(
        slug: impl Into<String>,
        doctype: impl Into<String>,
        context: impl Into<String>,
    ) -> Self {
        Self::Scoped {
            slug: slug.into(),
            doctype: doctype.into(),
            context: context.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn scoped_constructor_sets_fields() {
        let r = ResourceRef::scoped("hello-world", "task", "temper");
        match r {
            ResourceRef::Scoped {
                slug,
                doctype,
                context,
            } => {
                assert_eq!(slug, "hello-world");
                assert_eq!(doctype, "task");
                assert_eq!(context, "temper");
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
        let r = ResourceRef::scoped("foo", "task", "temper");
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
}
