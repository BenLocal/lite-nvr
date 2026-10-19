use super::*;

fn turns(n: usize, len: usize) -> Vec<Message> {
    (0..n)
        .flat_map(|i| {
            let text = format!("{i}").repeat(len);
            [Message::user(text.clone()), Message::assistant(text)]
        })
        .collect()
}

#[test]
fn keeps_history_within_limits() {
    let mut h = turns(3, 10);
    trim_history(&mut h, 8, 2000);
    assert_eq!(h.len(), 6);
}

#[test]
fn beyond_the_turn_limit_only_the_latest_turn_is_kept() {
    let mut h = turns(3, 1);
    trim_history(&mut h, 2, 2000);
    assert_eq!(h.len(), 2);
    assert_eq!(h[0].content, "2");
}

#[test]
fn beyond_the_char_budget_only_the_latest_turn_is_kept() {
    let mut h = turns(3, 10);
    trim_history(&mut h, 8, 45);
    assert_eq!(h.len(), 2);
    assert_eq!(h[0].content, "2".repeat(10));
}

#[test]
fn a_single_oversized_turn_is_dropped() {
    let mut h = turns(1, 100);
    trim_history(&mut h, 8, 50);
    assert!(h.is_empty());
}
