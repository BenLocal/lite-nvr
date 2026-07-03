use super::strip_punct;

#[test]
fn strip_punct_removes_cjk_and_ascii() {
    // Chinese sentence punctuation.
    assert_eq!(
        strip_punct("派饭时间，早上9点至下午5点。"),
        "派饭时间早上9点至下午5点"
    );
    assert_eq!(strip_punct("你好、世界；测试：完成！"), "你好世界测试完成");
    // ASCII punctuation (spaces are kept).
    assert_eq!(strip_punct("hello, world!"), "hello world");
}

#[test]
fn strip_punct_keeps_digits_and_letters() {
    assert_eq!(strip_punct("9点abc九"), "9点abc九");
}
