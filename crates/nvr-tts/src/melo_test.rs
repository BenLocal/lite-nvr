use super::*;

fn lexicon() -> Lexicon {
    let tokens: HashMap<String, i32> = [
        ("_", 0),
        ("n", 1),
        ("i", 2),
        ("h", 3),
        ("ao", 4),
        ("er", 5),
        ("sh", 6),
        ("hh", 7),
        ("ay", 8),
        ("v", 9),
        ("iy", 10),
        (",", 11),
        (".", 12),
        ("!", 13),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect();
    let lexicon =
        "你 n i 3 3\n好 h ao 3 3\n二 er 4\n十 sh i 2 2\nhi hh ay 7 7\nv v iy 7 7\nbad x 1\n";
    Lexicon::parse(lexicon, &tokens)
}

#[test]
fn chinese_per_character_with_blanks() {
    assert_eq!(
        lexicon().phones("你好"),
        Phones {
            phones: vec![0, 1, 0, 2, 0, 3, 0, 4, 0],
            tones: vec![0, 3, 0, 3, 0, 3, 0, 3, 0],
        }
    );
}

#[test]
fn punctuation_maps_to_ascii_and_unknowns_are_skipped() {
    let p = lexicon().phones("你，好！？");
    // 你 , 好 ! — "？" has no token in this lexicon and is dropped.
    assert_eq!(p.phones, vec![0, 1, 0, 2, 0, 11, 0, 3, 0, 4, 0, 13, 0]);
}

#[test]
fn english_words_and_spelled_fallback() {
    let lex = lexicon();
    assert_eq!(lex.phones("Hi").phones, vec![0, 7, 0, 8, 0]);
    // "vv" is not a word: spelled as v, v.
    assert_eq!(lex.phones("VV").phones, vec![0, 9, 0, 10, 0, 9, 0, 10, 0]);
}

#[test]
fn numbers_are_read_in_chinese() {
    assert_eq!(
        lexicon().phones("12").phones,
        lexicon().phones("十二").phones
    );
}

#[test]
fn entries_with_unknown_phones_are_ignored() {
    // "bad x 1" names a phone missing from tokens.txt, and b/a/d have no
    // entries either, so nothing but the blank remains.
    assert_eq!(lexicon().phones("bad").phones, vec![0]);
}
