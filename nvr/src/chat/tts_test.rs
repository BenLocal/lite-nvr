use super::*;

#[test]
fn prefers_the_npu_model_then_the_cpu_one() {
    let configs = configs(Path::new("/m"), "/lib/librknnrt.so");
    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].kind, ModelKind::MeloRknn);
    assert_eq!(configs[0].model_dir, Path::new("/m/MeloTTS-RKNN2"));
    assert_eq!(configs[0].rknn_lib, "/lib/librknnrt.so");
    assert_eq!(configs[1].kind, ModelKind::Vits);
    assert_eq!(configs[1].model_dir, Path::new("/m/vits-melo-tts-zh_en"));
}
