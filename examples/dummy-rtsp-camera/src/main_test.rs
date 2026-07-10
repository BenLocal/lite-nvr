use clap::Parser;

use super::*;

fn args(argv: &[&str]) -> Args {
    Args::parse_from(std::iter::once("dummy-rtsp-camera").chain(argv.iter().copied()))
}

#[test]
fn config_renders_server_and_file_source() {
    let args = args(&["--port", "9554", "--path", "/live/test1"]);
    let yaml = render_config(&args, std::path::Path::new("/tmp/clip.mp4"));
    assert_eq!(
        yaml,
        "server:\n  host: \"0.0.0.0\"\n  port: 9554\nmedia:\n  - name: \"dummy camera\"\n    path: \"/live/test1\"\n    kind: file\n    source: \"/tmp/clip.mp4\"\n"
    );
}

#[test]
fn mount_path_gets_leading_slash() {
    assert_eq!(normalize_path("live/test1"), "/live/test1");
    assert_eq!(normalize_path("/live/test1"), "/live/test1");
}

#[test]
fn clip_cache_name_encodes_generation_params() {
    let name = clip_cache_path(&args(&["--width", "640", "--height", "360", "--fps", "15"]));
    let name = name.file_name().unwrap().to_str().unwrap();
    assert_eq!(name, "dummy-rtsp-camera-640x360-15fps-ultrafast.mp4");
}

#[test]
fn ld_path_prepends_bundled_lib_dir() {
    let lib = std::path::Path::new("/repo/ffmpeg/lib");
    assert_eq!(prepend_ld_path(lib, None), "/repo/ffmpeg/lib");
    assert_eq!(
        prepend_ld_path(lib, Some("/usr/lib".into())),
        "/repo/ffmpeg/lib:/usr/lib"
    );
    assert_eq!(prepend_ld_path(lib, Some("".into())), "/repo/ffmpeg/lib");
}

#[tokio::test]
async fn explicit_ffmpeg_flag_is_honored_verbatim() {
    let a = args(&["--ffmpeg", "/opt/custom/ffmpeg"]);
    let ffmpeg = resolve_ffmpeg(&a).await.unwrap();
    assert_eq!(ffmpeg.bin, std::path::PathBuf::from("/opt/custom/ffmpeg"));
    assert!(ffmpeg.lib_dir.is_none());
}

#[test]
fn server_bin_falls_back_to_path_name() {
    let a = args(&[]);
    // Explicit flag wins over everything.
    let explicit = args(&["--server-bin", "/opt/oddity"]);
    assert_eq!(resolve_server_bin(&explicit), "/opt/oddity");
    // No flag: env or bare name; both acceptable here depending on test env.
    let resolved = resolve_server_bin(&a);
    assert!(!resolved.is_empty());
}
