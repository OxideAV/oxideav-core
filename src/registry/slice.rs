//! Distributed-slice auto-registration.
//!
//! Each sibling crate that ships a codec / container / filter / source
//! deposits a [`Registrar`] into the global [`REGISTRARS`] slice via the
//! [`crate::register!`] macro. [`RuntimeContext::with_all_features`]
//! walks the slice and invokes every registrar exactly once on the
//! `RuntimeContext` it's building.
//!
//! This replaces the historical umbrella-driven registration where
//! `oxideav::with_all_features()` enumerated every sibling explicitly.
//! With the slice, a sibling registers itself just by being linked into
//! the binary — no umbrella plumbing required. Consumers depend on the
//! sibling crates they want; pulling them in is enough.
//!
//! # Standalone opt-out
//!
//! Each sibling's `register!()` call lives behind that crate's
//! default-on `registry` cargo feature. Consumers that want the
//! standalone (no-`oxideav-core`-dep) build path turn the feature off
//! and the macro call disappears — no linkme entry, no slice
//! contribution.
//!
//! # Wasm
//!
//! linkme's distributed-slice machinery works on `wasm32-unknown-unknown`
//! and `wasm32-wasi` via wasm-ld's section grouping (recent stable
//! Rust toolchains ship a recent enough lld). The classic
//! `__attribute__((constructor))` path used by `ctor` / `inventory`
//! does NOT work on wasm; this is the deliberate reason linkme was
//! chosen over those.

use crate::RuntimeContext;

/// One entry in the distributed registrar slice. Carries the sibling
/// crate's display name (for the `with_all_features_traced` hook +
/// `with_all_features_filtered` opt-out filter) plus the function
/// pointer to invoke against the runtime context.
pub struct Registrar {
    /// Short display name for the sibling — typically the crate's
    /// shortened identifier (`"aac"`, `"vp8"`, `"webp"`, …). Used by
    /// the trace hook and the filter hook.
    pub name: &'static str,
    /// The register function. Takes a mutable reference to the
    /// runtime context being built; should install whatever codec /
    /// container / filter / source factories the sibling provides.
    pub func: fn(&mut RuntimeContext),
}

/// Global slice of registrars. Each sibling crate's
/// [`crate::register!`] call deposits one entry here; the slice is
/// materialised at link time.
#[linkme::distributed_slice]
pub static REGISTRARS: [Registrar];

impl RuntimeContext {
    /// Build a [`RuntimeContext`] populated by every sibling crate
    /// linked into the binary.
    ///
    /// Walks [`REGISTRARS`] and invokes each entry's `func` against a
    /// fresh context. Ordering is link-determined — siblings should
    /// not assume any particular order.
    pub fn with_all_features() -> Self {
        Self::with_all_features_traced(|_| {})
    }

    /// Like [`with_all_features`](Self::with_all_features) but invokes
    /// `trace(name)` immediately before each sibling's `register` fn.
    /// Useful for diagnosing register-time hangs — the last name passed
    /// to `trace` names the offending sibling.
    pub fn with_all_features_traced<F: FnMut(&str)>(mut trace: F) -> Self {
        let mut ctx = Self::new();
        for reg in REGISTRARS {
            trace(reg.name);
            (reg.func)(&mut ctx);
        }
        ctx
    }

    /// Like [`with_all_features`](Self::with_all_features) but skips
    /// entries whose name `filter` returns `false` for. Used by CLIs to
    /// implement opt-outs like `--no-hwaccel` (skip `videotoolbox` /
    /// `audiotoolbox`).
    pub fn with_all_features_filtered<F: FnMut(&str) -> bool>(mut filter: F) -> Self {
        let mut ctx = Self::new();
        for reg in REGISTRARS {
            if filter(reg.name) {
                (reg.func)(&mut ctx);
            }
        }
        ctx
    }
}

/// Auto-register a sibling crate's `register` fn into the global
/// [`REGISTRARS`] slice.
///
/// Place at module scope inside the sibling crate, gated behind the
/// crate's `registry` cargo feature so the standalone build path
/// stays decoupled from `oxideav-core`:
///
/// ```ignore
/// pub fn register(ctx: &mut oxideav_core::RuntimeContext) {
///     /* install factories */
/// }
///
/// #[cfg(feature = "registry")]
/// oxideav_core::register!("aac", register);
/// ```
///
/// The macro expands to a `#[linkme::distributed_slice]` static. The
/// sibling crate must have `linkme = "0.3"` in its `[dependencies]`
/// so the absolute path `::linkme::distributed_slice` resolves at
/// macro expansion time.
#[macro_export]
macro_rules! register {
    ($name:literal, $func:path) => {
        // `#[used]` keeps rustc from dropping the static during the
        // pre-link DCE pass; without it, integration tests that don't
        // directly reference the static drop the slice entry on the
        // floor and the slice walker observes an empty registry.
        #[used]
        #[::linkme::distributed_slice($crate::REGISTRARS)]
        static __OXIDEAV_AUTO_REGISTRAR: $crate::Registrar = $crate::Registrar {
            name: $name,
            func: $func,
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_walks_cleanly() {
        // The test binary doesn't link any sibling crates, so the slice
        // is empty here — `with_all_features` returns a fresh context
        // with no registrations.
        let ctx = RuntimeContext::with_all_features();
        assert_eq!(ctx.codecs.decoder_ids().count(), 0);
    }

    #[test]
    fn trace_hook_fires_zero_times_on_empty_registry() {
        let mut names = Vec::<String>::new();
        let _ctx = RuntimeContext::with_all_features_traced(|n| names.push(n.to_string()));
        assert!(names.is_empty());
    }

    #[test]
    fn filter_hook_fires_zero_times_on_empty_registry() {
        let mut names = Vec::<String>::new();
        let _ctx = RuntimeContext::with_all_features_filtered(|n| {
            names.push(n.to_string());
            true
        });
        assert!(names.is_empty());
    }
}
