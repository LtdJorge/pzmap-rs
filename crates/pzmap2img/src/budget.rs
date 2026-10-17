//! A limit on the memory the build phase's large allocations may use.
//!
//! Code charges the budget before it allocates. A charge that would go over the limit returns
//! [`BuildError::OutOfBudget`], so the build fails with an error instead of running out of
//! memory. For example, each cell is charged its lotpack's file size before it is parsed, and
//! the unused part is released once the real size is known. The tile and room name tables are
//! not counted.

use crate::{
    config::BuildLimits,
    error::{
        BuildError,
        Result,
    },
};
use std::sync::atomic::{
    AtomicU64,
    Ordering,
};

/// Tracks large allocations and rejects reservations that would exceed the memory limit.
#[derive(Debug)]
pub struct BuildBudget {
    ceiling: u64,
    charged: AtomicU64,
    high_water: AtomicU64,
}

impl BuildBudget {
    /// Creates a budget limited to `ceiling` bytes.
    #[must_use]
    pub fn new(ceiling: u64) -> Self {
        BuildBudget {
            ceiling,
            charged: AtomicU64::new(0),
            high_water: AtomicU64::new(0),
        }
    }

    /// Creates a budget with no practical limit.
    #[cfg(test)]
    #[must_use]
    pub fn unlimited() -> Self {
        BuildBudget::new(u64::MAX)
    }

    /// Reserves `bytes` before they are allocated. `what` names the allocation in the error.
    ///
    /// Two threads charging at once can't both get the last bytes under the limit.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::OutOfBudget`] if the charge would go over the limit. Nothing is
    /// charged in that case.
    pub fn charge(&self, what: &'static str, bytes: u64) -> Result<()> {
        let updated = self
            .charged
            .try_update(Ordering::AcqRel, Ordering::Acquire, |charged| {
                charged
                    .checked_add(bytes)
                    .filter(|next| *next <= self.ceiling)
            });

        match updated {
            // `try_update` returns the value before the update.
            Ok(previous) => {
                self.high_water
                    .fetch_max(previous.saturating_add(bytes), Ordering::Relaxed);
                Ok(())
            }
            Err(charged) => Err(BuildError::OutOfBudget {
                what,
                bytes,
                charged,
                ceiling: self.ceiling,
            }),
        }
    }

    /// Gives back `bytes` that were charged but are now free.
    ///
    /// Releasing more than is charged leaves the total at zero.
    pub fn release(&self, bytes: u64) {
        self.charged
            .try_update(Ordering::AcqRel, Ordering::Acquire, |charged| {
                Some(charged.saturating_sub(bytes))
            })
            .ok();
    }

    /// Returns the bytes currently charged.
    #[cfg(test)]
    #[must_use]
    pub fn charged(&self) -> u64 {
        self.charged.load(Ordering::Relaxed)
    }

    /// Returns the most that was ever charged at once.
    #[must_use]
    pub fn high_water(&self) -> u64 {
        self.high_water.load(Ordering::Relaxed)
    }
}

/// The memory budget and the build limits, bundled together.
#[derive(Copy, Clone, Debug)]
pub struct BuildCtx<'a> {
    /// The configured limits.
    pub limits: &'a BuildLimits,
    /// The shared memory budget.
    pub budget: &'a BuildBudget,
}

impl<'a> BuildCtx<'a> {
    /// Bundles `limits` and `budget`.
    #[must_use]
    pub fn new(limits: &'a BuildLimits, budget: &'a BuildBudget) -> Self {
        BuildCtx { limits, budget }
    }

    /// Reserves `bytes` for the allocation named `what`. See [`BuildBudget::charge`].
    pub fn charge(&self, what: &'static str, bytes: u64) -> Result<()> {
        self.budget.charge(what, bytes)
    }

    /// Gives back bytes that are now free. See [`BuildBudget::release`].
    pub fn release(&self, bytes: u64) {
        self.budget.release(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charges_accumulate_and_release() {
        let budget = BuildBudget::new(1000);

        budget.charge("a", 400).expect("400 fits");
        budget.charge("b", 400).expect("800 fits");
        assert_eq!(budget.charged(), 800);

        budget.release(400);
        assert_eq!(budget.charged(), 400);
        assert_eq!(budget.high_water(), 800, "the peak is not forgotten");
    }

    #[test]
    fn a_charge_over_the_ceiling_is_refused() {
        let budget = BuildBudget::new(1000);
        budget.charge("a", 900).expect("900 fits");

        let err = budget
            .charge("blob", 200)
            .expect_err("1100 does not fit in 1000");

        match err {
            BuildError::OutOfBudget {
                what,
                bytes,
                charged,
                ceiling,
            } => assert_eq!((what, bytes, charged, ceiling), ("blob", 200, 900, 1000)),
            other => panic!("expected OutOfBudget, got {other:?}"),
        }

        // The refused charge is not taken.
        assert_eq!(budget.charged(), 900);
    }

    #[test]
    fn an_overflowing_charge_is_refused_rather_than_wrapping() {
        let budget = BuildBudget::new(u64::MAX);
        budget.charge("a", 10).expect("10 fits");

        assert!(budget.charge("huge", u64::MAX).is_err());
        assert_eq!(budget.charged(), 10);
    }

    #[test]
    fn a_release_without_a_charge_saturates_at_zero() {
        let budget = BuildBudget::new(1000);
        budget.release(500);
        assert_eq!(budget.charged(), 0);
    }

    /// Two threads must not both get the last bytes under the limit.
    #[test]
    fn concurrent_charges_cannot_both_take_the_last_bytes() {
        for _ in 0..200 {
            let budget = BuildBudget::new(100);
            let winners = std::sync::atomic::AtomicU32::new(0);

            std::thread::scope(|scope| {
                for _ in 0..8 {
                    scope.spawn(|| {
                        if budget.charge("race", 60).is_ok() {
                            winners.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                }
            });

            assert_eq!(
                winners.load(Ordering::Relaxed),
                1,
                "exactly one 60-byte charge fits in 100 bytes"
            );
            assert_eq!(budget.charged(), 60);
        }
    }
}
