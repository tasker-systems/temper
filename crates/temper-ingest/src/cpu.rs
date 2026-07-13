//! Performance-core detection — the one place any surface asks "how many cores can
//! actually do fast work here?"
//!
//! This exists because ORT's `with_intra_threads(0)` ("let ORT size the pool") is a
//! **bad default on heterogeneous ARM**. Measured on a 12-core M4 Pro (8 performance +
//! 4 efficiency), embedding one 262 KB segment (task `019f57d2`):
//!
//! | intra-op threads | embed |
//! |---|---|
//! | `0` (ORT picks) | 10.77s — only ~598% CPU: ORT used ~6 threads |
//! | 8 (performance cores) | **9.62s** |
//! | 12 (all cores) | 9.73s |
//!
//! Two lessons, both counterintuitive, both encoded here:
//!
//! 1. `0` is not "all cores" — it is ORT's *guess*, and it leaves half the box idle.
//! 2. Using *all* cores is also worse than using only the performance cores: an
//!    intra-op batch advances at the speed of its slowest thread, so folding in 4
//!    efficiency cores drags every barrier.
//!
//! Hence: **the useful number is the performance-core count, not the core count.**
//!
//! Lives in `temper-ingest` (not in the CLI) so every surface — CLI today, temper-api
//! once task `019f5892` measures the server under concurrent load — asks the question
//! the same way and cannot drift.

/// The number of *performance* cores available, when we can determine it honestly.
///
/// `None` means "we don't know" — and callers must treat that as *"keep whatever
/// behavior you already had"*, never as an excuse to invent a number. That is why this
/// returns `Option` rather than falling back to a plausible-looking guess: a wrong
/// count here is a silent, machine-shaped performance bug.
///
/// - **macOS**: `hw.perflevel0.logicalcpu` — Apple's performance-core tier. (On Intel
///   Macs there are no perf levels and this key is absent, so we fall back to
///   `hw.physicalcpu`, which is the right answer on a homogeneous CPU.)
/// - **Everywhere else**: `None`. Not because other platforms don't matter, but because
///   we have **no measurements** there. On homogeneous x86 the two ideas coincide and
///   ORT's own default is already reasonable; inventing a Linux heuristic from Apple
///   Silicon data would repeat exactly the mistake this whole investigation was about.
///   When someone measures a big-little Linux box, this is the function to teach.
pub fn performance_cores() -> Option<usize> {
    #[cfg(target_os = "macos")]
    {
        sysctl_usize("hw.perflevel0.logicalcpu")
            .or_else(|| sysctl_usize("hw.physicalcpu"))
            .filter(|&n| n > 0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Read an integer `sysctl` by name. `None` if the key is absent (e.g.
/// `hw.perflevel0.*` on an Intel Mac) or the call fails — never a fabricated value.
#[cfg(target_os = "macos")]
fn sysctl_usize(name: &str) -> Option<usize> {
    let cname = std::ffi::CString::new(name).ok()?;
    let mut value: i32 = 0;
    let mut size = std::mem::size_of::<i32>();
    // SAFETY: `cname` is a valid NUL-terminated C string; `value`/`size` are valid,
    // correctly-sized out-params for an integer sysctl. We pass null for the new-value
    // arguments, so this is a read-only query.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            (&raw mut value).cast::<libc::c_void>(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || value <= 0 {
        return None;
    }
    Some(value as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn performance_cores_is_plausible_on_macos() {
        let p = performance_cores().expect("macOS must report a performance-core count");
        let total = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        assert!(p > 0, "performance-core count must be positive, got {p}");
        assert!(
            p <= total,
            "performance cores ({p}) cannot exceed total cores ({total})"
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn performance_cores_is_none_off_macos() {
        // Deliberately unmeasured elsewhere — callers must keep their existing behavior
        // rather than adopt a number nobody benchmarked. See this module's doc comment.
        assert_eq!(performance_cores(), None);
    }
}
