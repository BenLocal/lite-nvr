use super::*;

#[test]
fn cardinals() {
    for (n, read) in [
        (0, "零"),
        (5, "五"),
        (10, "十"),
        (12, "十二"),
        (20, "二十"),
        (22, "二十二"),
        (100, "一百"),
        (101, "一百零一"),
        (110, "一百一十"),
        (1001, "一千零一"),
        (2026, "二千零二十六"),
        (10_000, "一万"),
        (10_010, "一万零一十"),
        (120_000, "十二万"),
        (1_000_100, "一百万零一百"),
        (99_999_999, "九千九百九十九万九千九百九十九"),
    ] {
        assert_eq!(cardinal(n), read, "{n}");
    }
}

#[test]
fn numbers_in_text() {
    assert_eq!(normalize("气温22度"), "气温二十二度");
    assert_eq!(normalize("圆周率3.14"), "圆周率三点一四");
    assert_eq!(normalize("电话13800138000"), "电话一三八零零一三八零零零");
    assert_eq!(normalize("编号007"), "编号零零七");
    assert_eq!(
        normalize("版本2.、完"),
        "版本二.、完",
        "a dot without digits stays"
    );
    assert_eq!(normalize("没有数字"), "没有数字");
}
