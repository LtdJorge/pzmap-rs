//! A pool of reusable tile buffers shared by the render threads.
//!
//! Rendering allocates and frees buffers of the same few sizes again and again. The pool keeps
//! freed buffers and hands them out again instead. It never holds more memory than its budget:
//! when the budget is used up, [`CanvasPool::acquire`] returns [`PoolError::Exhausted`] instead
//! of allocating more.
//!
//! [`CanvasPool::budget_for`] works out how much memory building the in-memory levels needs,
//! and [`CanvasPool::for_walk`] makes a pool with at least that budget.

use std::{
    ops::{
        Deref,
        DerefMut,
    },
    sync::{
        Mutex,
        atomic::{
            AtomicU64,
            Ordering,
        },
    },
};

use thiserror::Error;

use crate::render::blend::{
    BPP,
    Canvas,
};

/// An error from a `CanvasPool`, the pool of reusable tile buffers.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PoolError {
    /// A new buffer would take the pool over its budget.
    #[error(
        "a {width}x{height} tile needs {needed} bytes and the pool already holds {resident} of \
         its {budget} byte budget"
    )]
    Exhausted {
        width: u32,
        height: u32,
        needed: u64,
        resident: u64,
        budget: u64,
    },

    /// The tile is too large for its size in bytes to fit in a `usize`.
    #[error("a {width}x{height} tile does not have a byte size")]
    Unrepresentable { width: u32, height: u32 },

    /// The budget is smaller than building the in-memory levels needs.
    #[error(
        "a walk {depth} levels deep over {tile_size} pixel tiles on {threads} threads needs {} \
         MiB, more than the {} MiB budget",
        needed.div_ceil(1 << 20),
        budget / (1 << 20)
    )]
    BelowWalkMinimum {
        threads: usize,
        tile_size: u32,
        depth: u32,
        needed: u64,
        budget: u64,
    },
}

/// The free buffers of one byte length.
struct SizeClass {
    len: usize,
    free: Vec<Box<[u8]>>,
}

/// Reusable tile buffers shared by the render threads, with a limit on the memory they use.
///
/// Free buffers are grouped by length in bytes. A tile can reuse any free buffer of the right
/// length, whatever the width and height it was used for before.
pub struct CanvasPool {
    classes: Mutex<Vec<SizeClass>>,
    budget: u64,
    /// Bytes allocated and not yet freed, whether lent out or waiting for reuse.
    resident: AtomicU64,
    lent: AtomicU64,
    high_water: AtomicU64,
    /// How many times `acquire` allocated a new buffer instead of reusing one.
    allocations: AtomicU64,
}

impl CanvasPool {
    /// The memory, in bytes, that building `depth` levels in memory needs: `threads * (depth + 2)`
    /// tiles.
    ///
    /// Each thread holds one partly built tile per level, the child tile being merged into it,
    /// and one more tile for cropping edge tiles.
    #[must_use]
    pub fn budget_for(threads: usize, tile_size: u32, depth: u32) -> u64 {
        let tile = u64::from(tile_size) * u64::from(tile_size) * BPP as u64;
        let per_thread = u64::from(depth).saturating_add(2);
        u64::try_from(threads)
            .unwrap_or(u64::MAX)
            .saturating_mul(per_thread)
            .saturating_mul(tile)
    }

    /// Extra memory, in bytes, that lets a small map's top levels build their children in
    /// parallel.
    ///
    /// The parallel case only applies to levels with fewer than `4 * threads` tiles. If every
    /// tile on those levels were partly built at once, they would hold at most
    /// `80/3 * threads` tiles. This only raises the limit. No memory is allocated until it is
    /// used.
    #[must_use]
    pub fn parallel_descent_surcharge(threads: usize, tile_size: u32) -> u64 {
        let tile = u64::from(tile_size) * u64::from(tile_size) * BPP as u64;
        u64::try_from(threads)
            .unwrap_or(u64::MAX)
            .saturating_mul(80)
            .div_ceil(3)
            .saturating_mul(tile)
    }

    /// A pool for building `depth` levels in memory, with `budget` bytes as its limit.
    ///
    /// # Errors
    ///
    /// Returns [`PoolError::BelowWalkMinimum`] if `budget` is below [`CanvasPool::budget_for`]
    /// for the same arguments.
    pub fn for_walk(
        threads: usize,
        tile_size: u32,
        depth: u32,
        budget: u64,
    ) -> Result<Self, PoolError> {
        let needed = CanvasPool::budget_for(threads, tile_size, depth);
        if budget < needed {
            return Err(PoolError::BelowWalkMinimum {
                threads,
                tile_size,
                depth,
                needed,
                budget,
            });
        }
        Ok(CanvasPool::with_budget(budget))
    }

    /// An empty pool that may allocate up to `budget` bytes.
    #[must_use]
    pub fn with_budget(budget: u64) -> Self {
        CanvasPool {
            classes: Mutex::new(Vec::new()),
            budget,
            resident: AtomicU64::new(0),
            lent: AtomicU64::new(0),
            high_water: AtomicU64::new(0),
            allocations: AtomicU64::new(0),
        }
    }

    /// A fully transparent `width` x `height` canvas, reusing a free buffer if there is one.
    ///
    /// # Errors
    ///
    /// Returns [`PoolError::Exhausted`] if there is no free buffer of the right size and a new
    /// one would go over the budget, and [`PoolError::Unrepresentable`] if the size in bytes
    /// overflows.
    pub fn acquire(&self, width: u32, height: u32) -> Result<PooledCanvas<'_>, PoolError> {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(BPP))
            .ok_or(PoolError::Unrepresentable { width, height })?;
        let needed = u64::try_from(len).unwrap_or(u64::MAX);

        let recycled = self.take_free(len);

        let pixels = match recycled {
            Some(mut pixels) => {
                pixels.fill(0);
                pixels
            }
            None => {
                // Only a new allocation adds to `resident`. The atomic update stops two threads
                // from both taking the last room in the budget.
                self.resident
                    .try_update(Ordering::AcqRel, Ordering::Acquire, |resident| {
                        resident
                            .checked_add(needed)
                            .filter(|next| *next <= self.budget)
                    })
                    .map_err(|resident| PoolError::Exhausted {
                        width,
                        height,
                        needed,
                        resident,
                        budget: self.budget,
                    })?;
                self.allocations.fetch_add(1, Ordering::Relaxed);
                vec![0_u8; len].into_boxed_slice()
            }
        };

        let lent = self.lent.fetch_add(needed, Ordering::Relaxed) + needed;
        self.high_water.fetch_max(lent, Ordering::Relaxed);

        let canvas = Canvas::from_pixels(width, height, pixels)
            .ok_or(PoolError::Unrepresentable { width, height })?;

        Ok(PooledCanvas { canvas, pool: self })
    }

    fn take_free(&self, len: usize) -> Option<Box<[u8]>> {
        let mut classes = self.classes.lock().ok()?;
        classes
            .iter_mut()
            .find(|class| class.len == len)
            .and_then(|class| class.free.pop())
    }

    fn release(&self, pixels: Box<[u8]>) {
        let len = pixels.len();
        self.lent
            .fetch_sub(u64::try_from(len).unwrap_or(0), Ordering::Relaxed);

        let Ok(mut classes) = self.classes.lock() else {
            // The lock is poisoned, so another thread panicked and the render is failing anyway.
            // Drop the buffer.
            return;
        };

        match classes.iter_mut().find(|class| class.len == len) {
            Some(class) => class.free.push(pixels),
            None => classes.push(SizeClass {
                len,
                free: vec![pixels],
            }),
        }
    }

    /// The most bytes the pool may allocate.
    #[must_use]
    pub const fn budget(&self) -> u64 {
        self.budget
    }

    /// Bytes allocated and not yet freed, whether lent out or free. Never more than
    /// [`CanvasPool::budget`].
    #[must_use]
    pub fn resident(&self) -> u64 {
        self.resident.load(Ordering::Relaxed)
    }

    /// The most bytes that were lent out at the same time.
    #[must_use]
    pub fn high_water(&self) -> u64 {
        self.high_water.load(Ordering::Relaxed)
    }

    /// How many times a new buffer was allocated instead of reused. If this keeps growing,
    /// buffers are not being returned.
    #[must_use]
    pub fn allocations(&self) -> u64 {
        self.allocations.load(Ordering::Relaxed)
    }
}

impl std::fmt::Debug for CanvasPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasPool")
            .field("budget", &self.budget)
            .field("resident", &self.resident())
            .field("lent", &self.lent.load(Ordering::Relaxed))
            .field("high_water", &self.high_water())
            .field("allocations", &self.allocations())
            .finish()
    }
}

/// A [`Canvas`] borrowed from a [`CanvasPool`]. Dropping it gives the buffer back.
///
/// Because this happens on drop, buffers come back even when a render fails partway through.
pub struct PooledCanvas<'a> {
    canvas: Canvas,
    pool: &'a CanvasPool,
}

impl Deref for PooledCanvas<'_> {
    type Target = Canvas;

    fn deref(&self) -> &Canvas {
        &self.canvas
    }
}

impl DerefMut for PooledCanvas<'_> {
    fn deref_mut(&mut self) -> &mut Canvas {
        &mut self.canvas
    }
}

impl Drop for PooledCanvas<'_> {
    fn drop(&mut self) {
        let pixels = std::mem::replace(&mut self.canvas, Canvas::empty()).into_pixels();
        if !pixels.is_empty() {
            self.pool.release(pixels);
        }
    }
}

impl std::fmt::Debug for PooledCanvas<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledCanvas")
            .field("width", &self.width())
            .field("height", &self.height())
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const TILE: u64 = 64 * 64 * 4;

    #[test]
    fn a_returned_buffer_is_the_one_handed_out_next() {
        let pool = CanvasPool::with_budget(TILE * 4);

        let first = pool.acquire(64, 64).unwrap().pixels().as_ptr();
        // The first canvas was dropped at the end of its statement, so this one must reuse it.
        let second = pool.acquire(64, 64).unwrap().pixels().as_ptr();

        assert_eq!(first, second);
        assert_eq!(pool.allocations(), 1);
        assert_eq!(pool.resident(), TILE);
    }

    #[test]
    fn a_recycled_buffer_is_transparent_again() {
        let pool = CanvasPool::with_budget(TILE * 2);

        {
            let mut canvas = pool.acquire(64, 64).unwrap();
            canvas.pixels_mut().fill(0xAB);
        }

        let canvas = pool.acquire(64, 64).unwrap();
        assert!(canvas.pixels().iter().all(|&b| b == 0));
    }

    #[test]
    fn the_budget_is_a_ceiling_and_not_a_suggestion() {
        let pool = CanvasPool::with_budget(TILE * 2);

        let _first = pool.acquire(64, 64).unwrap();
        let _second = pool.acquire(64, 64).unwrap();

        let third = pool.acquire(64, 64);
        assert!(matches!(third, Err(PoolError::Exhausted { .. })));
        assert_eq!(pool.resident(), TILE * 2);
    }

    /// A pool that ran out works again once a buffer comes back.
    #[test]
    fn a_pool_that_refused_still_works_once_a_buffer_comes_back() {
        let pool = CanvasPool::with_budget(TILE);

        let first = pool.acquire(64, 64).unwrap();
        assert!(pool.acquire(64, 64).is_err());

        drop(first);
        assert!(pool.acquire(64, 64).is_ok());
    }

    #[test]
    fn high_water_tracks_the_live_set_not_the_total() {
        let pool = CanvasPool::with_budget(TILE * 4);

        {
            let _a = pool.acquire(64, 64).unwrap();
            let _b = pool.acquire(64, 64).unwrap();
        }
        drop(pool.acquire(64, 64).unwrap());
        drop(pool.acquire(64, 64).unwrap());

        assert_eq!(pool.high_water(), TILE * 2);
        assert_eq!(pool.allocations(), 2);
    }

    #[test]
    fn a_budget_below_the_walk_minimum_is_refused() {
        let needed = CanvasPool::budget_for(4, 64, 2);
        assert!(matches!(
            CanvasPool::for_walk(4, 64, 2, needed - 1),
            Err(PoolError::BelowWalkMinimum { .. })
        ));
        assert!(CanvasPool::for_walk(4, 64, 2, needed).is_ok());
    }
}
