//! The home of a resource: exactly one of a context or a cognitive map.
//! Parse-don't-validate: surfaces resolve a ref into one variant before
//! building a `CreateResource` command — never a placeholder id plus a flag.

use serde::{Deserialize, Serialize};

use crate::types::ids::{CogmapId, ContextId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HomeAnchor {
    Context(ContextId),
    Cogmap(CogmapId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ids::{CogmapId, ContextId};

    #[test]
    fn home_anchor_serde_roundtrip() {
        let c = HomeAnchor::Context(ContextId::new());
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(c, serde_json::from_str(&j).unwrap());
        let m = HomeAnchor::Cogmap(CogmapId::new());
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(m, serde_json::from_str(&j).unwrap());
    }
}
