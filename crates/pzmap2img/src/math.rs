//! Integer division helpers for coordinate maths that stable Rust doesn't provide.

/// Divides rounding toward negative infinity.
///
/// Rust's `/` rounds toward zero, which puts a negative coordinate in the wrong tile. This is
/// the same as `i32::div_floor`, which is not yet stable.
///
/// # Panics
///
/// Panics if `rhs` is 0, or on `i32::MIN / -1`, the same as `/`.
#[inline]
pub(crate) const fn div_floor_i32(lhs: i32, rhs: i32) -> i32 {
    let q = lhs / rhs;
    let r = lhs % rhs;

    // The subtraction can't overflow. `q` can only be `i32::MIN` when `rhs` is 1 or -1, and
    // then `r` is 0, so this branch isn't taken.
    if (r > 0 && rhs < 0) || (r < 0 && rhs > 0) {
        q - 1
    } else {
        q
    }
}

/// Divides rounding toward positive infinity, the counterpart to [`div_floor_i32`].
///
/// # Panics
///
/// Panics if `rhs` is 0, or on `i32::MIN / -1`, the same as `/`.
#[inline]
pub(crate) const fn div_ceil_i32(lhs: i32, rhs: i32) -> i32 {
    let q = lhs / rhs;
    let r = lhs % rhs;

    // The addition can't overflow. `q` can only be `i32::MAX` when `rhs` is 1 or -1, and then
    // `r` is 0, so this branch isn't taken.
    if (r > 0 && rhs > 0) || (r < 0 && rhs < 0) {
        q + 1
    } else {
        q
    }
}

#[cfg(test)]
mod tests {
    use super::{
        div_ceil_i32,
        div_floor_i32,
    };

    #[test]
    fn rounds_toward_negative_infinity() {
        assert_eq!(div_floor_i32(8, 8), 1);
        assert_eq!(div_floor_i32(7, 8), 0);
        assert_eq!(div_floor_i32(0, 8), 0);

        assert_eq!(div_floor_i32(-1, 8), -1);
        assert_eq!(div_floor_i32(-9, 8), -2);

        assert_eq!(div_floor_i32(-8, 8), -1);
        assert_eq!(div_floor_i32(-16, 8), -2);

        assert_eq!(div_floor_i32(7, -8), -1);
        assert_eq!(div_floor_i32(-7, -8), 0);
        assert_eq!(div_floor_i32(8, -8), -1);

        assert_eq!(div_floor_i32(i32::MIN, 1), i32::MIN);
        assert_eq!(div_floor_i32(i32::MIN, 2), i32::MIN / 2);
        assert_eq!(div_floor_i32(i32::MAX, 1), i32::MAX);
        assert_eq!(div_floor_i32(i32::MAX, -1), -i32::MAX);
    }

    #[test]
    fn rounds_toward_positive_infinity() {
        assert_eq!(div_ceil_i32(8, 8), 1);
        assert_eq!(div_ceil_i32(9, 8), 2);
        assert_eq!(div_ceil_i32(0, 8), 0);

        assert_eq!(div_ceil_i32(-1, 8), 0);
        assert_eq!(div_ceil_i32(-9, 8), -1);

        assert_eq!(div_ceil_i32(-8, 8), -1);
        assert_eq!(div_ceil_i32(16, 8), 2);

        assert_eq!(div_ceil_i32(7, -8), 0);
        assert_eq!(div_ceil_i32(-7, -8), 1);

        assert_eq!(div_ceil_i32(i32::MIN, 1), i32::MIN);
        assert_eq!(div_ceil_i32(i32::MAX, 1), i32::MAX);
        assert_eq!(div_ceil_i32(i32::MAX, -1), -i32::MAX);
    }
}
