//! Unit tests for the reconnect backoff schedule (pure timing logic, no media).

use super::{RECONNECT_BASE, RECONNECT_MAX, backoff_delay};

#[test]
fn backoff_is_fast_right_after_a_drop() {
    // fails == 0 is a fresh drop (or right after a success): reconnect fast.
    assert_eq!(backoff_delay(0), RECONNECT_BASE);
    // The first failed open still retries at the base delay.
    assert_eq!(backoff_delay(1), RECONNECT_BASE);
}

#[test]
fn backoff_doubles_per_consecutive_failure() {
    assert_eq!(backoff_delay(2), RECONNECT_BASE * 2); // 6s
    assert_eq!(backoff_delay(3), RECONNECT_BASE * 4); // 12s
    assert_eq!(backoff_delay(4), RECONNECT_BASE * 8); // 24s
}

#[test]
fn backoff_is_capped_and_never_overflows() {
    // 3s * 16 = 48s would exceed the ceiling, so it clamps.
    assert_eq!(backoff_delay(5), RECONNECT_MAX);
    // A very long dead source stays clamped, and the shift never overflows.
    assert_eq!(backoff_delay(1000), RECONNECT_MAX);
    assert_eq!(backoff_delay(u32::MAX), RECONNECT_MAX);
}
