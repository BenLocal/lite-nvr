//! Unit tests for the live source pool: `Director::add_source` /
//! `remove_source` update pool membership (switchability) and clear region
//! slots. Pure state logic — no media, no running compositor.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::{Director, LayoutState, LatestFrame, Region, geoms_of, slots_of};

/// Build a `Director` over the given regions with the given source ids in the
/// pool (each with an empty frame cell), bypassing the compositor run loop.
fn director_with(regions: &[Region], pool_ids: &[&str]) -> Director {
    let state = Arc::new(Mutex::new(LayoutState {
        geoms: geoms_of(regions),
        slots: slots_of(regions),
        generation: 0,
    }));
    let pool: HashMap<String, LatestFrame> = pool_ids
        .iter()
        .map(|id| (id.to_string(), Arc::new(Mutex::new(None))))
        .collect();
    Director {
        state,
        pool: Arc::new(Mutex::new(pool)),
    }
}

fn region(source_id: &str) -> Region {
    Region {
        source_id: source_id.to_string(),
        x: 0,
        y: 0,
        w: 100,
        h: 100,
    }
}

#[test]
fn add_source_makes_it_switchable() {
    let regions = [region("a")];
    let d = director_with(&regions, &["a"]);

    // "b" is not in the pool yet, so a switch to it is rejected.
    assert!(!d.switch(0, "b"));

    d.add_source("b".to_string(), Arc::new(Mutex::new(None)));

    // Now it is in the pool and can be shown.
    assert!(d.switch(0, "b"));
    assert_eq!(d.active(), vec!["b".to_string()]);
}

#[test]
fn remove_source_drops_from_pool_and_clears_its_slots() {
    let regions = [region("a"), region("b"), region("a")];
    let d = director_with(&regions, &["a", "b"]);

    d.remove_source("a");

    // Every slot that showed "a" is cleared to black (""); "b" is untouched.
    assert_eq!(
        d.active(),
        vec!["".to_string(), "b".to_string(), "".to_string()]
    );

    // "a" is no longer switchable; "b" still is.
    assert!(!d.switch(0, "a"));
    assert!(d.switch(0, "b"));
}

#[test]
fn empty_source_id_always_switchable_to_black() {
    let regions = [region("a")];
    let d = director_with(&regions, &["a"]);
    // Clearing a region to black needs no pool membership.
    assert!(d.switch(0, ""));
    assert_eq!(d.active(), vec!["".to_string()]);
}
