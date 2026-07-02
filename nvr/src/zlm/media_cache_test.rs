use super::*;

#[test]
fn regist_marks_live_unregist_clears() {
    let c = MediaCache::new();
    assert!(!c.is_live("rtp", "cam1"));
    c.on_regist("rtp", "cam1");
    assert!(c.is_live("rtp", "cam1"));
    assert!(!c.is_live("rtp", "cam2"));
    assert!(!c.is_live("live", "cam1")); // app is part of the key
    c.on_unregist("rtp", "cam1");
    assert!(!c.is_live("rtp", "cam1"));
}

#[test]
fn live_streams_lists_current() {
    let c = MediaCache::new();
    c.on_regist("rtp", "cam1");
    c.on_regist("rtp", "cam2");
    c.on_unregist("rtp", "cam1");
    let mut got = c.live_streams();
    got.sort();
    assert_eq!(got, vec![("rtp".to_string(), "cam2".to_string())]);
}
