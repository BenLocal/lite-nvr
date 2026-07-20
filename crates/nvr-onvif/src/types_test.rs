use super::*;

#[test]
fn velocity_clamps_each_axis() {
    let v = PtzVelocity::new(2.0, -3.0, 0.5);
    assert_eq!(
        v,
        PtzVelocity {
            pan: 1.0,
            tilt: -1.0,
            zoom: 0.5
        }
    );
}

#[test]
fn error_display_is_human_readable() {
    assert_eq!(format!("{}", OnvifError::Auth), "authentication rejected");
    assert_eq!(
        format!("{}", OnvifError::NoProfile("P1".into())),
        "profile not found: P1"
    );
}
