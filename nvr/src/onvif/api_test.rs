use super::*;

#[test]
fn maps_direction_verbs_to_actions() {
    assert_eq!(resolve_ptz("stop", 128, None), Some(PtzAction::Stop));
    assert_eq!(
        resolve_ptz("preset_call", 0, Some("P2")),
        Some(PtzAction::Preset("P2".into()))
    );
    // preset_call needs a token
    assert_eq!(resolve_ptz("preset_call", 0, None), None);
    // unknown verb
    assert_eq!(resolve_ptz("wat", 128, None), None);

    // left at speed 255 -> pan -1.0; right -> +1.0; up -> tilt +1.0
    match resolve_ptz("left", 255, None).unwrap() {
        PtzAction::Move(v) => {
            assert!((v.pan + 1.0).abs() < 1e-6);
            assert_eq!(v.tilt, 0.0);
            assert_eq!(v.zoom, 0.0);
        }
        _ => panic!("expected Move"),
    }
    match resolve_ptz("zoom_in", 255, None).unwrap() {
        PtzAction::Move(v) => assert!((v.zoom - 1.0).abs() < 1e-6),
        _ => panic!("expected Move"),
    }
}
