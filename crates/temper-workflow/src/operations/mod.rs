//! Operations layer — the opinionated frame every door and the server share: the managed-meta
//! actions (defaults, merges, validation), ref parsing and the act surface.
//!
//! The server's write-command vocabulary is not here: it lives in `temper-services`, the only
//! crate that runs it.

mod actions;
mod refs;
mod surface;

pub use actions::{
    apply_defaults, apply_defaults_value, assemble_frontmatter_document, merge_managed_meta,
    merge_open_meta, stamp_provenance, strip_system_managed_fields, validate_doctype,
    validate_managed_meta, validate_open_meta_keys, validate_slug, ActionError,
    FrontmatterIdentity, ValidateManagedMetaParams,
};
pub use refs::{
    decorated_ref, decorated_ref_bounded, is_remote_url, parse_ref, resolve_provenance_source,
    sluggify,
};
pub use surface::{
    InProcessSurface, RelayedSurface, Surface, RELAYED_SURFACE_HEADER, SERVICE_CREDENTIAL_HEADER,
    SURFACE_HEADER,
};

#[cfg(test)]
mod smoke {
    /// Smoke test: the module compiles and is reachable.
    #[test]
    fn module_exists() {
        // No-op; existence of this test passing means the module compiled.
    }
}
