use nvr_rkllm::Role;
use serde_json::json;

use super::*;

fn parse(body: Value) -> CompletionRequest {
    serde_json::from_value(body).unwrap()
}

#[test]
fn converts_messages_and_parameters() {
    let req = parse(json!({
        "messages": [
            {"role": "system", "content": "be brief"},
            {"role": "user", "content": [{"type": "text", "text": "hi"}, {"type": "text", "text": "there"}]},
            {"role": "assistant", "content": "hello"},
            {"role": "user", "content": "1+1?"}
        ],
        "max_tokens": 10,
        "max_completion_tokens": 20,
        "temperature": 0.2,
        "top_k": 5,
        "chat_template_kwargs": {"enable_thinking": true}
    }));
    let chat = req.into_chat().unwrap();
    let roles: Vec<Role> = chat.messages.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        [Role::System, Role::User, Role::Assistant, Role::User]
    );
    assert_eq!(chat.messages[1].content, "hi\nthere");
    assert_eq!(chat.max_tokens, Some(20), "max_completion_tokens wins");
    assert_eq!(chat.sampling.temperature, Some(0.2));
    assert_eq!(chat.sampling.top_k, Some(5));
    assert!(chat.enable_thinking);
}

#[test]
fn rejects_what_the_runtime_cannot_do() {
    for (body, needle) in [
        (json!({"messages": [], }), "empty"),
        (
            json!({"messages": [{"role": "user", "content": "x"}], "stop": ["\n"]}),
            "stop",
        ),
        (
            json!({"messages": [{"role": "user", "content": "x"}], "tools": []}),
            "tools",
        ),
        (
            json!({"messages": [{"role": "user", "content": "x"}], "n": 2}),
            "n = 1",
        ),
        (
            json!({"messages": [{"role": "tool", "content": "x"}]}),
            "role",
        ),
        (
            json!({"messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "data:"}}]}]}),
            "text content",
        ),
    ] {
        let err = parse(body).into_chat().unwrap_err();
        assert!(err.contains(needle), "{err}");
    }
    // Explicit nulls are fine.
    let ok = parse(json!({"messages": [{"role": "user", "content": "x"}], "stop": null, "n": 1}));
    assert!(ok.into_chat().is_ok());
}

fn completion() -> Completion {
    Completion {
        id: "chatcmpl-1".into(),
        created: 7,
        model: "qwen".into(),
    }
}

fn done(reason: FinishReason) -> Delta {
    Delta::Done {
        usage: Usage {
            prompt_tokens: 3,
            completion_tokens: 2,
        },
        finish_reason: reason,
    }
}

#[test]
fn non_streaming_response_joins_the_deltas() {
    let body = completion()
        .response(vec![
            Delta::Reasoning("想".into()),
            Delta::Content("答".into()),
            Delta::Content("案".into()),
            done(FinishReason::Length),
        ])
        .unwrap();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["choices"][0]["message"]["content"], "答案");
    assert_eq!(body["choices"][0]["message"]["reasoning_content"], "想");
    assert_eq!(body["choices"][0]["finish_reason"], "length");
    assert_eq!(body["usage"]["total_tokens"], 5);

    let plain = completion()
        .response(vec![Delta::Content("a".into()), done(FinishReason::Stop)])
        .unwrap();
    assert!(
        plain["choices"][0]["message"]
            .get("reasoning_content")
            .is_none()
    );

    assert_eq!(
        completion().response(vec![Delta::Error("npu".into())]),
        Err("npu".to_string())
    );
}

#[test]
fn stream_chunks_follow_openai() {
    let c = completion();
    assert_eq!(c.role_chunk()["choices"][0]["delta"]["role"], "assistant");
    let chunk = &c.chunks(Delta::Content("x".into()), false)[0];
    assert_eq!(chunk["object"], "chat.completion.chunk");
    assert_eq!(chunk["choices"][0]["delta"]["content"], "x");
    assert!(chunk["choices"][0]["finish_reason"].is_null());
    assert_eq!(
        c.chunks(Delta::Reasoning("r".into()), false)[0]["choices"][0]["delta"]["reasoning_content"],
        "r"
    );

    let end = c.chunks(done(FinishReason::Stop), false);
    assert_eq!(end.len(), 1);
    assert_eq!(end[0]["choices"][0]["finish_reason"], "stop");
    let end = c.chunks(done(FinishReason::Stop), true);
    assert_eq!(end.len(), 2);
    assert_eq!(end[1]["choices"], json!([]));
    assert_eq!(end[1]["usage"]["completion_tokens"], 2);

    assert_eq!(
        c.chunks(Delta::Error("boom".into()), false)[0]["error"]["message"],
        "boom"
    );
}
