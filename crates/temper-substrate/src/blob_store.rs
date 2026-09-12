//! The provider seam for blob bytes (spec: binary blobs, 2026-09-01 — D1/D4/D6).
//!
//! The substrate never links a provider SDK: the commit path only needs to know that the
//! provider holds the object at its content-addressed pathname (D4's gate — the SQL wrapper
//! cannot ask the provider). The real client (`VercelBlobStore`) lives in temper-services,
//! which owns upload/read-through; tests supply an in-memory fake. The trait grows when a
//! surface needs it to; it does not pre-provision delete or a multipart protocol.

use anyhow::Result;
use bytes::Bytes;
use futures_core::Stream;
use std::pin::Pin;

/// The streamed body of a read-through `get`: the native shape of both the provider client's
/// response and the Axum response body, so bytes cross the read path without re-buffering
/// (D6 — the API is the only reader, and streams rather than buffers).
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

/// What a successful `put` reports. Deliberately pathname-shaped, not URL-shaped: the
/// content-addressed pathname IS the object's identity (D1), and provider URL formats never
/// leak into the substrate.
#[derive(Debug, Clone)]
pub struct PutReceipt {
    pub pathname: String,
}

/// What `head` reports about an object present at its content-addressed pathname. The
/// metadata a read-through response needs to set its own headers (D6: stream with the
/// stored content type).
#[derive(Debug, Clone)]
pub struct BlobHead {
    pub content_type: Option<String>,
    pub content_bytes: i64,
}

/// Derive the content-addressed pathname for a bare sha256 hex hash: `{hash[0:2]}/{hash}` — the
/// git shape (D1). One derivation, shared by the Rust fire path and mirrored (as a verification,
/// never a re-derivation) by the SQL wrapper.
pub fn blob_pathname(content_hash: &str) -> String {
    format!("{}/{}", &content_hash[..2], content_hash)
}

/// The provider CDN's cache window for committed blob objects — one year, immutable. One
/// definition, here beside `put`'s signature: the commit path (including the under-lock
/// restore, task `01a09360-e00a-7d90-858d-f4998dd70b6c`) and the read-through response
/// headers must not be able to drift apart.
pub const IMMUTABLE_CACHE_MAX_AGE: u32 = 365 * 24 * 60 * 60;

/// External object storage, as far as the substrate's commit path is concerned.
///
/// Landed as RPITIT (`impl Future + Send` desugaring) when the only method was `exists`;
/// the surfaces task grew it (D6/D7) and its consumers need `Arc<dyn BlobStore>` (AppState,
/// handler injection), which requires object safety. `#[async_trait]` — the repo's incumbent
/// shape for exactly this, see `CredentialBroker` — boxes each future: the allocation is
/// noise next to provider I/O, and the Send guarantee the RPITIT shape existed to make is
/// what `async_trait`'s default produces.
#[async_trait::async_trait]
pub trait BlobStore: Send + Sync + std::fmt::Debug {
    /// Does the provider hold an object at this content-addressed pathname? The commit gate:
    /// a blob_committed event whose bytes are absent from the provider is refused before the
    /// ledger ever sees it (D4: verify, then append + project).
    async fn exists(&self, pathname: &str) -> Result<bool>;

    /// Upload bytes to the content-addressed pathname (D7's single-request path — the caller
    /// has already applied the cap; the provider cannot know the vocabulary it enforces).
    /// `cache_control_max_age` is the provider CDN's cache window for the object; an
    /// implementation applies the window the caller asks — a silently discarded parameter
    /// would make the commit path's posture a fiction.
    async fn put(
        &self,
        pathname: &str,
        content_type: &str,
        body: Bytes,
        cache_control_max_age: u32,
    ) -> Result<PutReceipt>;

    /// Stream the object's bytes back (D6 read-through). `consistent` asks the provider to
    /// bypass its CDN cache — irrelevant for content-addressed objects in practice (immutable
    /// bytes), but the escape hatch the erasure task needs for post-delete verification.
    async fn get(&self, pathname: &str, consistent: bool) -> Result<ByteStream>;

    /// Metadata for an object, or `None` when the provider holds none there. The read path's
    /// cheap pre-flight: visibility is checked in SQL first; this answers "are the bytes
    /// actually there" without pulling them.
    async fn head(&self, pathname: &str) -> Result<Option<BlobHead>>;

    /// Strike the bytes at each content-addressed pathname. Idempotent — a pathname the
    /// provider does not hold is a successful no-op, so an at-least-once caller (the erasure
    /// queue's retry fence) needs no existence guard — and batched: one call for the whole
    /// list, never a per-pathname loop (the provider's array form). The ONLY byte-level
    /// destructive verb: bytes only ever strike, there is no byte-level soft state
    /// (`no-byte-soft-delete`). Ruled 2026-09-06 (the delete-act design): one batch verb
    /// serves both emptying acts — the ordinary delete's post-commit release and the erasure
    /// queue's drain.
    ///
    /// There is deliberately no `list`: a drain knows its pathnames (derived, never
    /// enumerated), and enumeration would make this seam an erasure-surface.
    async fn delete(&self, pathnames: &[&str]) -> Result<()>;
}

/// In-memory fake for integration tests: the caller pre-registers the pathnames it has
/// "uploaded", and reads/writes hit that map — the same contract the real client satisfies.
/// Interior mutability (`Mutex`) because the trait's methods take `&self`, the shape a
/// shared `Arc<dyn BlobStore>` demands.
#[derive(Default)]
pub struct InMemoryBlobStore {
    objects: std::sync::Mutex<std::collections::HashMap<String, (String, Bytes)>>,
}

impl InMemoryBlobStore {
    pub fn with_object(mut self, pathname: impl Into<String>) -> Self {
        self.objects.get_mut().unwrap().insert(
            pathname.into(),
            ("application/octet-stream".into(), Bytes::new()),
        );
        self
    }

    /// Pre-register an "uploaded" object without consuming `self` — the shape a test helper
    /// needs when the same store must stay live across a commit, a strike, and a re-commit.
    pub fn insert(&self, pathname: impl Into<String>) {
        self.objects
            .lock()
            .unwrap()
            .entry(pathname.into())
            .or_insert_with(|| ("application/octet-stream".into(), Bytes::new()));
    }

    pub fn contains(&self, pathname: &str) -> bool {
        self.objects.lock().unwrap().contains_key(pathname)
    }
}

impl std::fmt::Debug for InMemoryBlobStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryBlobStore")
            .field("objects", &self.objects.lock().unwrap().len())
            .finish()
    }
}

#[async_trait::async_trait]
impl BlobStore for InMemoryBlobStore {
    async fn exists(&self, pathname: &str) -> Result<bool> {
        Ok(self.objects.lock().unwrap().contains_key(pathname))
    }

    async fn put(
        &self,
        pathname: &str,
        content_type: &str,
        body: Bytes,
        _cache_control_max_age: u32,
    ) -> Result<PutReceipt> {
        self.objects
            .lock()
            .unwrap()
            .insert(pathname.to_string(), (content_type.to_string(), body));
        Ok(PutReceipt {
            pathname: pathname.to_string(),
        })
    }

    async fn get(&self, pathname: &str, _consistent: bool) -> Result<ByteStream> {
        let bytes = self
            .objects
            .lock()
            .unwrap()
            .get(pathname)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| anyhow::anyhow!("in-memory provider holds no object at {pathname}"))?;
        Ok(Box::pin(futures_util::stream::once(
            async move { Ok(bytes) },
        )))
    }

    async fn head(&self, pathname: &str) -> Result<Option<BlobHead>> {
        Ok(self
            .objects
            .lock()
            .unwrap()
            .get(pathname)
            .map(|(content_type, bytes)| BlobHead {
                content_type: Some(content_type.clone()),
                content_bytes: bytes.len() as i64,
            }))
    }

    async fn delete(&self, pathnames: &[&str]) -> Result<()> {
        let mut objects = self.objects.lock().unwrap();
        for pathname in pathnames {
            // Absent ⇒ no-op: the trait's idempotence contract, so the caller's retry of a
            // partially-landed batch cannot fail on the pathname it already struck.
            objects.remove(*pathname);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // FAILS IF: the fake's delete loses the trait's contract — batched (one call for the
    // list), idempotent (an absent pathname is a no-op, never an error), and total (every
    // named pathname is gone, nothing else).
    #[tokio::test]
    async fn delete_strikes_every_named_pathname_and_tolerates_an_absent_one() {
        let store = InMemoryBlobStore::default()
            .with_object("ab/aa")
            .with_object("cd/bb");
        assert!(store.contains("ab/aa") && store.contains("cd/bb"));

        store.delete(&["ab/aa", "zz/absent"]).await.unwrap();

        assert!(!store.contains("ab/aa"), "the named pathname is struck");
        assert!(store.contains("cd/bb"), "the unnamed pathname survives");
        // Idempotence: the retry — same list, the first already gone — succeeds.
        store.delete(&["ab/aa", "zz/absent"]).await.unwrap();
        assert!(store.contains("cd/bb"));
    }
}
