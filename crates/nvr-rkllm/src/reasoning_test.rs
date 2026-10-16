use super::*;

fn split(starts_in_reasoning: bool, chunks: &[&str]) -> (String, String) {
    let mut s = ThinkSplitter::new(starts_in_reasoning);
    let mut deltas: Vec<Delta> = chunks.iter().flat_map(|c| s.push(c)).collect();
    deltas.extend(s.finish());
    let (mut reasoning, mut content) = (String::new(), String::new());
    for d in deltas {
        match d {
            Delta::Reasoning(t) => reasoning.push_str(&t),
            Delta::Content(t) => content.push_str(&t),
            other => panic!("unexpected {other:?}"),
        }
    }
    (reasoning, content)
}

#[test]
fn plain_answer_is_all_content() {
    assert_eq!(
        split(false, &["你好", "，世界"]),
        (String::new(), "你好，世界".into())
    );
}

#[test]
fn tags_split_across_chunks_are_recognized() {
    let (r, c) = split(
        false,
        &["<thi", "nk>\n想一想", "</th", "ink>\n\n答案", "是 2"],
    );
    assert_eq!(r, "想一想");
    assert_eq!(c, "答案是 2");
}

#[test]
fn output_that_starts_inside_reasoning() {
    let (r, c) = split(true, &["先算", "</think>", "\n\n2"]);
    assert_eq!(r, "先算");
    assert_eq!(c, "2");
}

#[test]
fn a_lone_angle_bracket_is_not_swallowed() {
    let (_, c) = split(false, &["a <", " b"]);
    assert_eq!(c, "a < b");
    let (_, c) = split(false, &["x<t"]);
    assert_eq!(c, "x<t", "held-back prefix is flushed at the end");
}
