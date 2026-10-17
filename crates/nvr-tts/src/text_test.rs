use super::*;

fn split(chunks: &[&str]) -> Vec<String> {
    let mut s = SentenceSplitter::new();
    let mut out: Vec<String> = chunks.iter().flat_map(|c| s.push(c)).collect();
    out.extend(s.finish());
    out
}

#[test]
fn splits_chinese_sentences_as_they_complete() {
    let mut s = SentenceSplitter::new();
    assert!(s.push("今天天气").is_empty());
    assert_eq!(s.push("很好。明天"), vec!["今天天气很好。"]);
    assert_eq!(s.push("会下雨吗？我"), vec!["明天会下雨吗？"]);
    assert_eq!(s.finish(), Some("我".into()));
}

#[test]
fn closing_quotes_and_repeated_marks_stay_with_the_sentence() {
    assert_eq!(
        split(&["他说：“好！”然后", "走了。。"]),
        vec!["他说：“好！”", "然后走了。。"]
    );
    assert_eq!(split(&["真的吗?!", "是的"]), vec!["真的吗?!", "是的"]);
}

#[test]
fn english_periods_need_a_following_space() {
    assert_eq!(
        split(&["Pi is 3.14 roughly. ", "Done"]),
        vec!["Pi is 3.14 roughly.", "Done"]
    );
    // A trailing period waits for the next character before deciding.
    let mut s = SentenceSplitter::new();
    assert!(s.push("Version 2.").is_empty());
    assert!(s.push("5 is out").is_empty());
    assert_eq!(s.finish(), Some("Version 2.5 is out".into()));
}

#[test]
fn long_clauses_are_cut_at_the_last_comma() {
    let long = "这是一个非常非常长的句子，它没有句号，但是一直在说话，说了很多很多很多很多很多的内容，还在继续";
    let mut s = SentenceSplitter::new();
    let out = s.push(long);
    assert_eq!(out.len(), 1);
    assert!(out[0].ends_with('，'), "{out:?}");
    assert_eq!(s.finish(), Some("还在继续".into()));
}

#[test]
fn unspeakable_sentences_are_dropped() {
    assert_eq!(split(&["好的。\n\n---\n", "。"]), vec!["好的。"]);
}

#[test]
fn markdown_links_and_emoji_are_not_read() {
    assert_eq!(clean("## **北京**是*中国*的首都 🎉"), "北京是中国的首都");
    assert_eq!(clean("- 第一点\n2. 第二点"), "第一点 第二点");
    assert_eq!(
        clean("详见[官网](https://example.com)或 https://x.y/z 。"),
        "详见官网或 。"
    );
    assert_eq!(clean("```rust\nlet x = 1;\n```"), "let x = 1;");
    assert_eq!(clean("> 引用\u{0}文字"), "引用 文字");
    assert_eq!(clean("3 * 4 = 12"), "3 4 = 12");
}
