use nvr_asr::Transcript;
use serde_json::json;

use super::payload;

#[test]
fn partial_payload_carries_id_and_open_flag() {
    let (event, data) = payload(
        "cam1",
        &Transcript::Partial {
            segment: 7,
            text: "今天".into(),
        },
    );
    assert_eq!(event, "partial");
    assert_eq!(
        data,
        json!({ "pipe": "cam1", "id": 7, "final": false, "text": "今天" })
    );
}

#[test]
fn final_payload_splits_sentences() {
    let (event, data) = payload(
        "cam1",
        &Transcript::Final {
            segment: 7,
            text: "你好。再见！".into(),
            start: 1.0,
            duration: 2.0,
        },
    );
    assert_eq!(event, "final");
    assert_eq!(data["id"], 7);
    assert_eq!(data["final"], true);
    assert_eq!(data["text"], "你好。再见！");
    let sentences = data["sentences"].as_array().unwrap();
    assert_eq!(sentences.len(), 2);
    assert_eq!(sentences[0]["text"], "你好。");
    assert_eq!(sentences[1]["text"], "再见！");
    assert_eq!(sentences[1]["start"], 2.0);
}
