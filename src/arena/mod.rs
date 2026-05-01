//! Refcounted arena pool for decoder frame allocations.
//!
//! This module is the runtime half of the DoS-protection framework
//! described in [`crate::limits`]. It provides three types:
//!
//! - [`ArenaPool`] — a pool of reusable raw byte buffers (`Box<[u8]>`)
//!   that a decoder leases from. Pool size and per-buffer capacity are
//!   fixed at construction; together they bound peak RSS by
//!   construction (`max_arenas × cap_per_arena`).
//!
//! - [`Arena`] — a single buffer leased from the pool. Allocations are
//!   bump-pointer (no per-alloc bookkeeping, no fragmentation). When
//!   the `Arena` is dropped, its buffer is returned to the pool, *not*
//!   freed — this is what makes the pool memory-reusing rather than
//!   memory-leaking. If the pool has been dropped before the arena
//!   (last-arena-outlives-pool), the arena's buffer is freed normally.
//!
//! - [`Frame`] / [`FrameInner`] — a refcounted (`Rc<FrameInner>`)
//!   handle that holds an `Arena` plus per-plane offset/length pairs
//!   and a small [`FrameHeader`]. As long as any clone of a `Frame`
//!   exists, its arena (and therefore its buffer) stays out of the
//!   pool. The last `Drop` returns the buffer.
//!
//! ## Design choices for round 1
//!
//! - **Hand-rolled bump allocator** inside a `Box<[u8]>`. We
//!   deliberately do not depend on the `bumpalo` crate yet — the
//!   logic is twenty lines and avoids pulling in a dependency before
//!   profiling justifies it. The signature is intentionally compatible
//!   with what a `bumpalo`-backed implementation would look like, so
//!   swapping later is a contained refactor.
//!
//! - **`Rc` for `Frame`, not `Arc`.** This module targets the
//!   single-threaded decode path (one decoder, one consumer thread).
//!   The bump-pointer cursor is `Cell<usize>` for the same reason
//!   (no atomics on the hot path). For the cross-thread decode path
//!   — where a decoder produces frames on one thread and a consumer
//!   reads them on another — see the sibling [`sync`] module, which
//!   mirrors this API 1:1 with `Arc<FrameInner>` / atomic cursor so
//!   `Frame: Send + Sync`.
//!
//! - **`Arena::alloc<T>` returns `&mut [T]` borrowed from the arena.**
//!   The borrow is bounded by the lifetime of the `&Arena` reference,
//!   not the lifetime of the arena itself; the arena's buffer is
//!   held inside an `UnsafeCell` so multiple calls to `alloc` against
//!   the same `&Arena` can each carve out non-overlapping sub-slices.
//!   This matches `bumpalo::Bump::alloc_slice_*` semantics.

pub mod sync;

use std::cell::{Cell, UnsafeCell};
use std::mem::{align_of, size_of};
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};

use crate::error::{Error, Result};
use crate::format::PixelFormat;

/// Pool of reusable byte buffers for arena-backed frame allocations.
///
/// Construct one per decoder via [`ArenaPool::new`]. Lease an
/// [`Arena`] per frame via [`ArenaPool::lease`]; drop the arena (or
/// drop the last clone of a [`Frame`] holding it) to return its
/// buffer to the pool.
///
/// **Backpressure:** when all `max_arenas` slots are checked out the
/// next [`ArenaPool::lease`] returns
/// [`Error::ResourceExhausted`]. A decoder that hits this should
/// surface the error to its caller rather than busy-loop — the
/// upstream pipeline is supposed to drop frames it no longer needs,
/// which returns a buffer to the pool.
///
/// `ArenaPool` is `Send + Sync` (the inner `Mutex<Vec<…>>` makes it
/// safe to share across threads even though [`Arena`] / [`Frame`]
/// themselves are `!Send` due to their `Rc`/`Cell` contents). This
/// asymmetry is intentional: a parallel-decoder thread can share a
/// single pool while each thread owns its own arenas — see also the
/// sibling [`sync::ArenaPool`] whose leases are themselves `Send + Sync`.
pub struct ArenaPool {
    inner: Mutex<PoolInner>,
    cap_per_arena: usize,
    max_arenas: usize,
    max_alloc_count_per_arena: u32,
}

struct PoolInner {
    /// Buffers currently sitting idle in the pool (ready to lease).
    idle: Vec<Box<[u8]>>,
    /// Total buffers ever allocated by this pool (idle + in-flight).
    /// Caps lazy growth at `max_arenas`.
    total_allocated: usize,
}

impl ArenaPool {
    /// Construct a new pool with `max_arenas` buffer slots, each of
    /// `cap_per_arena` bytes. Buffers are allocated lazily on first
    /// lease — a freshly constructed pool holds no memory.
    ///
    /// Per-arena allocation count is capped at `max_alloc_count` (use
    /// [`ArenaPool::new`] which defaults to a generous 1M, or
    /// [`ArenaPool::with_alloc_count_cap`] to tighten further).
    pub fn new(max_arenas: usize, cap_per_arena: usize) -> Arc<Self> {
        Self::with_alloc_count_cap(max_arenas, cap_per_arena, 1_000_000)
    }

    /// Like [`ArenaPool::new`] but lets the caller set the per-arena
    /// allocation-count cap. Useful when the caller is plumbing
    /// [`crate::DecoderLimits`] through.
    pub fn with_alloc_count_cap(
        max_arenas: usize,
        cap_per_arena: usize,
        max_alloc_count_per_arena: u32,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(PoolInner {
                idle: Vec::with_capacity(max_arenas),
                total_allocated: 0,
            }),
            cap_per_arena,
            max_arenas,
            max_alloc_count_per_arena,
        })
    }

    /// Capacity of each arena buffer this pool hands out, in bytes.
    pub fn cap_per_arena(&self) -> usize {
        self.cap_per_arena
    }

    /// Maximum number of arenas that may be checked out at once.
    pub fn max_arenas(&self) -> usize {
        self.max_arenas
    }

    /// Lease one arena from the pool. Returns
    /// [`Error::ResourceExhausted`] if every arena slot is already
    /// checked out by an [`Arena`] (or a [`Frame`] holding one).
    pub fn lease(self: &Arc<Self>) -> Result<Arena> {
        let buffer = {
            let mut inner = self.inner.lock().expect("ArenaPool mutex poisoned");
            if let Some(buf) = inner.idle.pop() {
                buf
            } else if inner.total_allocated < self.max_arenas {
                inner.total_allocated += 1;
                vec![0u8; self.cap_per_arena].into_boxed_slice()
            } else {
                return Err(Error::resource_exhausted(format!(
                    "ArenaPool exhausted: all {} arenas checked out",
                    self.max_arenas
                )));
            }
        };

        Ok(Arena {
            buffer: UnsafeCell::new(buffer),
            cursor: Cell::new(0),
            alloc_count: Cell::new(0),
            cap: self.cap_per_arena,
            alloc_count_cap: self.max_alloc_count_per_arena,
            pool: Arc::downgrade(self),
        })
    }

    /// Return a buffer to the idle list. Called from `Arena::Drop`;
    /// not part of the public API.
    fn release(&self, buffer: Box<[u8]>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.idle.push(buffer);
        }
        // If the lock is poisoned, drop the buffer normally — the
        // pool is in an unusable state already.
    }
}

/// One leased buffer from an [`ArenaPool`].
///
/// Allocations are bump-pointer: each call to [`Arena::alloc`] carves
/// out a fresh aligned slice from the head of the buffer. There is no
/// per-allocation header and no individual free — the entire arena
/// is reset (returned to the pool) only when the `Arena` is dropped.
///
/// `Arena` is `!Send + !Sync` because its bump cursor is a `Cell` and
/// its buffer is an `UnsafeCell` accessed without locks. This is
/// fine for the round-1 single-threaded decoder path. A future
/// parallel-decoder variant can use `AtomicUsize` for the cursor and
/// regain `Send`.
pub struct Arena {
    /// Backing buffer leased from the pool. Wrapped in `UnsafeCell`
    /// so `&Arena::alloc` can return `&mut [T]` slices that borrow
    /// non-overlapping ranges of the same buffer.
    buffer: UnsafeCell<Box<[u8]>>,
    /// Bump cursor: the next free byte offset within `buffer`.
    cursor: Cell<usize>,
    /// Number of allocations performed so far.
    alloc_count: Cell<u32>,
    /// Cached cap (== `pool.cap_per_arena` at lease time).
    cap: usize,
    /// Cached cap (== `pool.max_alloc_count_per_arena` at lease time).
    alloc_count_cap: u32,
    /// Weak handle back to the pool so `Drop` can return the buffer.
    pool: Weak<ArenaPool>,
}

impl Arena {
    /// Capacity of this arena in bytes.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Bytes consumed by allocations so far.
    pub fn used(&self) -> usize {
        self.cursor.get()
    }

    /// Number of allocations performed so far.
    pub fn alloc_count(&self) -> u32 {
        self.alloc_count.get()
    }

    /// `true` once the per-arena allocation-count cap has been
    /// reached. Decoders that produce many small allocations should
    /// poll this and bail with [`Error::ResourceExhausted`] when it
    /// flips, instead of waiting for the next [`Arena::alloc`] call
    /// to fail.
    pub fn alloc_count_exceeded(&self) -> bool {
        self.alloc_count.get() >= self.alloc_count_cap
    }

    /// Allocate `count` `T`s out of this arena. Returns a borrowed
    /// `&mut [T]` (lifetime bounded by the borrow of `self`) initialised
    /// to `T::default()` for `Default`-implementing primitive types
    /// — actually, we leave the bytes untouched and rely on the type
    /// `T` being a plain integer/byte type. **The caller is
    /// responsible for fully writing the returned slice before reading
    /// it.** This matches the "decoder fills it, then never re-reads
    /// uninitialised bytes" pattern.
    ///
    /// Returns [`Error::ResourceExhausted`] if either the per-arena
    /// byte cap or the per-arena allocation-count cap would be
    /// exceeded.
    ///
    /// # Safety / contract
    ///
    /// `T` must be a "plain old data" type with no `Drop` glue and
    /// no invariants that need a constructor — typically `u8`, `i16`,
    /// `u32`, `f32`, etc. The arena does not run destructors on
    /// allocated values. This is enforced via a `T: Copy` bound.
    ///
    /// **Aliasing model:** the bump cursor is monotonically
    /// non-decreasing, so successive `alloc` calls return slices
    /// covering disjoint regions of the arena's internal `UnsafeCell`.
    /// This is the standard arena-allocator pattern (cf.
    /// `bumpalo::Bump::alloc_slice_*`) and is the reason this method
    /// takes `&self` rather than `&mut self`.
    #[allow(clippy::mut_from_ref)] // see "Aliasing model" doc above.
    pub fn alloc<T>(&self, count: usize) -> Result<&mut [T]>
    where
        T: Copy,
    {
        // Allocation-count cap.
        let next_count =
            self.alloc_count.get().checked_add(1).ok_or_else(|| {
                Error::resource_exhausted("Arena alloc_count overflow".to_string())
            })?;
        if next_count > self.alloc_count_cap {
            return Err(Error::resource_exhausted(format!(
                "Arena alloc-count cap of {} exceeded",
                self.alloc_count_cap
            )));
        }

        let elem_size = size_of::<T>();
        let elem_align = align_of::<T>();
        // Bytes requested.
        let bytes = elem_size
            .checked_mul(count)
            .ok_or_else(|| Error::resource_exhausted("Arena alloc size overflow".to_string()))?;

        // Align cursor up to T's alignment.
        let cursor = self.cursor.get();
        let aligned = align_up(cursor, elem_align).ok_or_else(|| {
            Error::resource_exhausted("Arena cursor alignment overflow".to_string())
        })?;
        let new_cursor = aligned.checked_add(bytes).ok_or_else(|| {
            Error::resource_exhausted("Arena cursor advance overflow".to_string())
        })?;

        if new_cursor > self.cap {
            return Err(Error::resource_exhausted(format!(
                "Arena cap of {} bytes exceeded (would consume {} bytes)",
                self.cap, new_cursor
            )));
        }

        // SAFETY: we hold &self, the buffer lives inside the arena
        // for the duration of `&self`, and we return a slice covering
        // `aligned..aligned+bytes` which we have just verified does
        // not overlap any previously-handed-out range (cursor is
        // monotonically non-decreasing). T: Copy guarantees we don't
        // need to drop the previous contents.
        let slice: &mut [T] = unsafe {
            let buf_ptr = (*self.buffer.get()).as_mut_ptr();
            let elem_ptr = buf_ptr.add(aligned).cast::<T>();
            std::slice::from_raw_parts_mut(elem_ptr, count)
        };

        self.cursor.set(new_cursor);
        self.alloc_count.set(next_count);
        Ok(slice)
    }

    /// Reset the arena to empty without releasing its buffer to the
    /// pool. Useful for a decoder that wants to reuse the same arena
    /// across several intermediate stages of the same frame. Callers
    /// must ensure no slice previously returned from [`Arena::alloc`]
    /// is still in use — Rust's borrow checker enforces this, since
    /// `reset` takes `&mut self`.
    pub fn reset(&mut self) {
        self.cursor.set(0);
        self.alloc_count.set(0);
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        // Take the buffer out of the UnsafeCell. We're in Drop, so
        // no other references to it can exist.
        let buffer = std::mem::replace(
            unsafe { &mut *self.buffer.get() },
            Vec::new().into_boxed_slice(),
        );
        if let Some(pool) = self.pool.upgrade() {
            pool.release(buffer);
        }
        // else: pool was dropped before us — buffer drops here.
    }
}

/// Round `n` up to the next multiple of `align`. `align` must be a
/// power of two. Returns `None` on overflow.
fn align_up(n: usize, align: usize) -> Option<usize> {
    debug_assert!(align.is_power_of_two(), "alignment must be a power of two");
    let mask = align - 1;
    n.checked_add(mask).map(|m| m & !mask)
}

/// Per-frame metadata carried alongside an [`Arena`] inside a
/// [`Frame`]. Kept minimal in round 1; round 2 will extend with
/// stride/colorspace/HDR fields as decoders need them.
///
/// `Copy` so it travels through the hot path with no allocation.
#[non_exhaustive]
#[derive(Copy, Clone, Debug)]
pub struct FrameHeader {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    /// Presentation timestamp in stream time-base units. `None` when
    /// the codec did not surface one (e.g. a still image).
    pub presentation_timestamp: Option<i64>,
}

impl FrameHeader {
    /// Construct a header with all four mandatory fields set. Use
    /// functional-update syntax (`FrameHeader { ..header }`) to add
    /// future fields safely.
    pub fn new(
        width: u32,
        height: u32,
        pixel_format: PixelFormat,
        presentation_timestamp: Option<i64>,
    ) -> Self {
        Self {
            width,
            height,
            pixel_format,
            presentation_timestamp,
        }
    }
}

/// Maximum number of planes a [`FrameInner`] can describe in round 1.
/// Covers every real-world video pixel format (1 plane for packed
/// RGB/YUV 4:2:2, 3 planes for I420/YV12/I444, 4 planes for YUVA / RGBA
/// planar). Audio is handled by a separate sibling type in a future
/// round; this module is video-only for now.
pub const MAX_PLANES: usize = 4;

/// The owned body of a refcounted [`Frame`].
///
/// Holds an [`Arena`] (the bytes), a fixed-size table of
/// `(offset_in_arena, length_in_bytes)` pairs (one per plane), and a
/// [`FrameHeader`]. The `plane_count` field tracks how many entries of
/// `plane_offsets` are actually populated. Up to [`MAX_PLANES`] planes
/// are supported.
///
/// **Lifetime:** an `Arena` returns its buffer to the pool when
/// dropped. A `Rc<FrameInner>` keeps the arena alive via its single
/// owned field, so as long as any clone of a [`Frame`] exists the
/// underlying buffer stays out of the pool.
pub struct FrameInner {
    arena: Arena,
    plane_offsets: [(usize, usize); MAX_PLANES],
    plane_count: u8,
    header: FrameHeader,
}

/// Refcounted handle to a decoded video frame. Construct via
/// [`Frame::new`]; clone freely (each clone bumps the refcount by 1).
/// The arena and its buffer are released back to the pool when the
/// last clone is dropped.
///
/// `Frame` is `Rc<FrameInner>` (single-threaded decoder path). For the
/// cross-thread decode path — where the consumer runs on a different
/// thread from the decoder — use the sibling [`sync::Frame`] which is
/// `Arc<sync::FrameInner>` and is `Send + Sync`.
pub type Frame = Rc<FrameInner>;

impl FrameInner {
    /// Construct a `Frame` (refcounted `Rc<FrameInner>`) from an arena,
    /// a slice of `(offset, length)` plane descriptors, and a header.
    /// Returns [`Error::InvalidData`] if more than [`MAX_PLANES`]
    /// planes are supplied or if any plane range falls outside the
    /// arena's used region.
    pub fn new(arena: Arena, planes: &[(usize, usize)], header: FrameHeader) -> Result<Frame> {
        if planes.len() > MAX_PLANES {
            return Err(Error::invalid(format!(
                "FrameInner supports at most {} planes (got {})",
                MAX_PLANES,
                planes.len()
            )));
        }
        let used = arena.used();
        for (i, (off, len)) in planes.iter().enumerate() {
            let end = off
                .checked_add(*len)
                .ok_or_else(|| Error::invalid(format!("plane {i}: offset+len overflow")))?;
            if end > used {
                return Err(Error::invalid(format!(
                    "plane {i}: range {off}..{end} exceeds arena used={used}"
                )));
            }
        }
        let mut plane_offsets = [(0usize, 0usize); MAX_PLANES];
        for (i, p) in planes.iter().enumerate() {
            plane_offsets[i] = *p;
        }
        Ok(Rc::new(FrameInner {
            arena,
            plane_offsets,
            plane_count: planes.len() as u8,
            header,
        }))
    }

    /// Number of planes this frame holds.
    pub fn plane_count(&self) -> usize {
        self.plane_count as usize
    }

    /// Read-only access to plane `i`. Returns `None` if `i` is out of
    /// range.
    pub fn plane(&self, i: usize) -> Option<&[u8]> {
        if i >= self.plane_count as usize {
            return None;
        }
        let (off, len) = self.plane_offsets[i];
        // SAFETY: plane ranges were validated against `arena.used()`
        // at construction; the arena's buffer has not changed since.
        // We borrow with the lifetime of `&self`.
        let buf: &[u8] = unsafe {
            let buf_ref = &*self.arena.buffer.get();
            &(**buf_ref)[off..off + len]
        };
        Some(buf)
    }

    /// Frame header (width / height / pixel format / pts).
    pub fn header(&self) -> &FrameHeader {
        &self.header
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_pool(slots: usize, cap: usize) -> Arc<ArenaPool> {
        ArenaPool::new(slots, cap)
    }

    #[test]
    fn pool_lease_returns_err_when_exhausted() {
        let pool = small_pool(2, 1024);
        let a = pool.lease().expect("first lease");
        let b = pool.lease().expect("second lease");
        let third = pool.lease();
        assert!(matches!(third, Err(Error::ResourceExhausted(_))));
        // Keep a and b alive past the assertion so they aren't dropped
        // before the failing lease.
        drop((a, b));
    }

    #[test]
    fn arena_alloc_caps_at_size_limit() {
        let pool = small_pool(1, 64);
        let arena = pool.lease().unwrap();
        // 64 bytes capacity. Allocate 32 u8s — fits.
        let _: &mut [u8] = arena.alloc::<u8>(32).unwrap();
        // Allocate another 32 u8s — exactly fills.
        let _: &mut [u8] = arena.alloc::<u8>(32).unwrap();
        // Any further allocation fails.
        let third = arena.alloc::<u8>(1);
        assert!(matches!(third, Err(Error::ResourceExhausted(_))));
    }

    #[test]
    fn arena_alloc_count_cap_fires() {
        let pool = ArenaPool::with_alloc_count_cap(1, 1024, 3);
        let arena = pool.lease().unwrap();
        let _: &mut [u8] = arena.alloc::<u8>(1).unwrap();
        let _: &mut [u8] = arena.alloc::<u8>(1).unwrap();
        let _: &mut [u8] = arena.alloc::<u8>(1).unwrap();
        assert!(arena.alloc_count_exceeded());
        let fourth = arena.alloc::<u8>(1);
        assert!(matches!(fourth, Err(Error::ResourceExhausted(_))));
    }

    #[test]
    fn arena_returns_to_pool_on_drop() {
        let pool = small_pool(1, 256);
        {
            let arena = pool.lease().expect("first lease");
            // Sanity: arena is leased; further leases would fail.
            assert!(matches!(pool.lease(), Err(Error::ResourceExhausted(_))));
            drop(arena);
        }
        // Arena dropped — pool slot must be free again.
        let _again = pool.lease().expect("re-lease after drop");
    }

    #[cfg(miri)]
    #[test]
    fn arena_alloc_can_return_misaligned_typed_slice() {
        let pool = small_pool(1, 0);
        let arena = pool.lease().unwrap();

        // Memory-safety issue: the arena's backing allocation is a
        // `Box<[u8]>`, so its base pointer is only guaranteed to be
        // byte-aligned. `alloc::<T>` aligns only the byte offset, not
        // the absolute address, and then constructs `&mut [T]`. The
        // empty-buffer case makes this deterministic: even an empty
        // `&mut [u32]` must have an aligned pointer, but `Box<[u8]>`
        // uses an alignment-1 dangling pointer when its length is 0.
        let _s: &mut [u32] = arena.alloc::<u32>(0).unwrap();
    }

    #[cfg(miri)]
    #[test]
    fn arena_alloc_allows_invalid_bit_patterns_for_copy_types() {
        let pool = small_pool(1, 1);
        let arena = pool.lease().unwrap();

        // Memory-safety issue: `alloc<T>` is a safe API but accepts any
        // `T: Copy`. Fresh pool buffers are zero-filled, and zero is not
        // a valid `NonZeroU8`. Reading through the returned reference
        // makes Miri report an invalid value created by safe code.
        let values = arena.alloc::<std::num::NonZeroU8>(1).unwrap();
        let _ = values[0].get();
    }

    #[cfg(miri)]
    #[test]
    fn arena_alloc_second_slice_invalidates_first_mut_reference() {
        let pool = small_pool(1, 2);
        let arena = pool.lease().unwrap();

        // Memory-safety issue: each `alloc` calls `[u8]::as_mut_ptr` on
        // the whole backing slice before carving out the requested
        // subslice. That materializes a new mutable borrow of the whole
        // buffer and invalidates previously returned `&mut` slices, even
        // when the byte ranges are disjoint.
        let first = arena.alloc::<u8>(1).unwrap();
        let second = arena.alloc::<u8>(1).unwrap();
        first[0] = 1;
        second[0] = 2;
    }

    fn build_simple_frame(pool: &Arc<ArenaPool>) -> Frame {
        let arena = pool.lease().unwrap();
        // Allocate 16 bytes for plane 0.
        let plane0: &mut [u8] = arena.alloc::<u8>(16).unwrap();
        for (i, b) in plane0.iter_mut().enumerate() {
            *b = i as u8;
        }
        // The slice borrowed from arena ends here.
        let header = FrameHeader::new(4, 4, PixelFormat::Gray8, Some(42));
        FrameInner::new(arena, &[(0, 16)], header).unwrap()
    }

    #[test]
    fn frame_refcount_keeps_arena_alive() {
        let pool = small_pool(1, 256);
        let frame = build_simple_frame(&pool);
        let clone = Rc::clone(&frame);
        drop(frame);
        // Clone is still valid; arena still leased.
        let plane = clone.plane(0).expect("plane 0");
        assert_eq!(plane.len(), 16);
        for (i, b) in plane.iter().enumerate() {
            assert_eq!(*b, i as u8);
        }
        assert_eq!(clone.header().width, 4);
        assert_eq!(clone.header().height, 4);
        assert_eq!(clone.header().presentation_timestamp, Some(42));
        // Pool still exhausted because clone holds the arena.
        assert!(matches!(pool.lease(), Err(Error::ResourceExhausted(_))));
    }

    #[test]
    fn last_drop_returns_arena_to_pool() {
        let pool = small_pool(1, 256);
        let frame = build_simple_frame(&pool);
        let clone = Rc::clone(&frame);
        drop(frame);
        drop(clone);
        // All clones gone — buffer must be back in the pool.
        let _again = pool.lease().expect("lease after last drop");
    }

    #[test]
    fn frame_rejects_too_many_planes() {
        let pool = small_pool(1, 256);
        let arena = pool.lease().unwrap();
        let header = FrameHeader::new(1, 1, PixelFormat::Gray8, None);
        let too_many = vec![(0usize, 0usize); MAX_PLANES + 1];
        let r = FrameInner::new(arena, &too_many, header);
        assert!(matches!(r, Err(Error::InvalidData(_))));
    }

    #[test]
    fn frame_rejects_plane_outside_arena() {
        let pool = small_pool(1, 64);
        let arena = pool.lease().unwrap();
        // arena.used() == 0; any non-empty plane is out of range.
        let header = FrameHeader::new(1, 1, PixelFormat::Gray8, None);
        let r = FrameInner::new(arena, &[(0, 16)], header);
        assert!(matches!(r, Err(Error::InvalidData(_))));
    }

    #[test]
    fn pool_outlives_buffer_drop_when_pool_dropped_first() {
        // Exotic: arena outlives its pool. Buffer just frees normally.
        let pool = small_pool(1, 64);
        let arena = pool.lease().unwrap();
        drop(pool);
        // Drop arena — must not panic. The Weak handle won't upgrade.
        drop(arena);
    }

    #[test]
    fn arena_reset_clears_allocations() {
        let pool = small_pool(1, 32);
        let mut arena = pool.lease().unwrap();
        let _: &mut [u8] = arena.alloc::<u8>(32).unwrap();
        // Cap reached.
        assert!(matches!(
            arena.alloc::<u8>(1),
            Err(Error::ResourceExhausted(_))
        ));
        arena.reset();
        // After reset we can allocate again.
        let _: &mut [u8] = arena.alloc::<u8>(32).unwrap();
    }
}
