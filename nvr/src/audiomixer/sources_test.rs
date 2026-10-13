use super::*;

#[test]
fn physical_inputs_include_terminal_and_both_hdmi_capture_cards() {
    let cards = "0 [rockchiphdmi0 ]: output\n1 [rockchiphdmi1 ]: output\n2 [rockchipes8388 ]: terminal\n3 [D6911codec ]: hdmi\n4 [rockchiphdmiin ]: hdmi";
    let pcm = "00-00: output : playback 1\n01-00: output : playback 1\n02-00: terminal : playback 1 : capture 1\n03-00: hdmi : capture 1\n04-00: hdmi : capture 1";
    let inputs = physical_inputs(cards, pcm);
    assert_eq!(
        inputs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [TERMINAL_SOURCE, "alsa:D6911codec", "alsa:rockchiphdmiin"]
    );
    assert_eq!(
        capture_address("alsa:D6911codec").as_deref(),
        Some("hw:CARD=D6911codec,DEV=0")
    );
    assert!(capture_address("alsa:rockchiphdmi0").is_none());
}

#[test]
fn capture_detection_uses_card_names_when_kernel_card_indices_change() {
    let cards = "1 [rockchipes8388 ]: terminal\n2 [D6911codec ]: hdmi\n3 [rockchiphdmiin ]: hdmi";
    let pcm = "01-00: terminal : capture 1\n02-00: hdmi : playback 1\n03-00: hdmi : capture 1";
    let inputs = physical_inputs(cards, pcm);
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].id, TERMINAL_SOURCE);
    assert_eq!(inputs[1].id, "alsa:rockchiphdmiin");
    assert!(physical_inputs("", "").is_empty());
}
