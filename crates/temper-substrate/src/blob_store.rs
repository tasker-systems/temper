//! The provider seam for blob bytes (spec: binary blobs, 2026-09-01 — D1/D4/D6).
//!
//! The substrate never links a provider SDK: the commit path only needs to know that the
//! provider holds the object at its content-addressed pathname (D4's gate — the SQL wrapper
//! cannot ask the provider), and the surfaces task owns upload/read-through, where the real
//! client lands. Tests supply an in-memory fake. The trait grows when a surface needs it to;
//! it does not pre-provision put/get/del.

use anyhow::Result;

/// Derive the content-addressed pathname for a bare sha256 hex hash: `{hash[0:2]}/{hash}` — the
/// git shape (D1). One derivation, shared by the Rust fire path and mirrored (as a verification,
/// never a re-derivation) by the SQL wrapper.
pub fn blob_pathname(content_hash: &str) -> String {
    format!("{}/{}", &content_hash[..2], content_hash)
}

/// External object storage, as far as the substrate's commit path is concerned.
pub trait BlobStore: Send + Sync {
    /// Does the provider hold an object at this content-addressed pathname? The commit gate:
    /// a blob_committed event whose bytes are absent from the provider is refused before the
    /// ledger ever sees it (D4: verify, then append + project).
    ///
    /// Desugared with an explicit `Send` bound (the `async_fn_in_trait` lint's suggested form):
    /// callers hold this future across `await` inside tokio workers, so the bound must be
    /// guaranteed by the trait, not inferred per implementation.
    fn exists(&self, pathname: &str) -> impl std::future::Future<Output = Result<bool>> + Send;
}

/// In-memory fake for integration tests: the caller pre-registers the pathnames it has
/// "uploaded", and `exists` reads that set — the same contract the real client satisfies.
#[derive(Default)]
pub struct InMemoryBlobStore {
    objects: std::collections::HashSet<String>,
}

impl InMemoryBlobStore {
    pub fn with_object(mut self, pathname: impl Into<String>) -> Self {
        self.objects.insert(pathname.into());
        self
    }

    pub fn contains(&self, pathname: &str) -> bool {
        self.objects.contains(pathname)
    }
}

impl BlobStore for InMemoryBlobStore {
    async fn exists(&self, pathname: &str) -> Result<bool> {
        Ok(self.objects.contains(pathname))
    }
}
