use super::*;

#[test]
fn tb_to_us_90khz() {
    // 90000 ticks at tb 1/90000 == 1s == 1_000_000 us
    assert_eq!(tb_to_us(90_000, 1, 90_000), 1_000_000);
}

#[test]
fn us_to_tb_90khz() {
    assert_eq!(us_to_tb(1_000_000, 1, 90_000), 90_000);
}

#[test]
fn round_trip_us_tb() {
    let tb = (1, 48_000);
    let us = tb_to_us(24_000, tb.0, tb.1); // 0.5s
    assert_eq!(us, 500_000);
    assert_eq!(us_to_tb(us, tb.0, tb.1), 24_000);
}

#[test]
fn duration_seconds_basic_and_clamped() {
    assert_eq!(duration_seconds(0, 10_000_000), 10.0);
    assert_eq!(duration_seconds(500, 100), 0.0);
}
