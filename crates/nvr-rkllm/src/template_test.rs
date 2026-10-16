use super::*;

fn chat() -> Vec<Message> {
    vec![
        Message::system("你是助手"),
        Message::user("你好"),
        Message::assistant("你好！"),
        Message::user("1+1=?"),
    ]
}

#[test]
fn qwen_renders_chatml_with_thinking_closed() {
    let p = Template::Qwen.render(&chat(), false);
    assert_eq!(
        p.text,
        "<|im_start|>system\n你是助手<|im_end|>\n\
         <|im_start|>user\n你好<|im_end|>\n\
         <|im_start|>assistant\n你好！<|im_end|>\n\
         <|im_start|>user\n1+1=?<|im_end|>\n\
         <|im_start|>assistant\n<think>\n\n</think>\n\n"
    );
    assert!(!p.starts_in_reasoning);
}

#[test]
fn qwen_with_thinking_lets_the_model_open_the_section() {
    let p = Template::Qwen.render(&[Message::user("hi")], true);
    assert!(p.text.ends_with("<|im_start|>assistant\n"));
    assert!(!p.starts_in_reasoning);
}

#[test]
fn deepseek_r1_starts_inside_the_think_section() {
    let p = Template::DeepseekR1.render(&chat(), true);
    assert_eq!(
        p.text,
        "你是助手<｜User｜>你好<｜Assistant｜>你好！<｜end▁of▁sentence｜>\
         <｜User｜>1+1=?<｜Assistant｜><think>\n"
    );
    assert!(p.starts_in_reasoning);
    let p = Template::DeepseekR1.render(&chat(), false);
    assert!(p.text.ends_with("<｜Assistant｜><think>\n\n</think>\n\n"));
    assert!(!p.starts_in_reasoning);
}

#[test]
fn template_names_parse() {
    assert_eq!("qwen".parse::<Template>().unwrap(), Template::Qwen);
    assert_eq!(
        "deepseek-r1".parse::<Template>().unwrap(),
        Template::DeepseekR1
    );
    assert!("llama".parse::<Template>().is_err());
}
