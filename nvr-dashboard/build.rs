use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

const SOURCE_INPUTS: &[&str] = &[
    "app/src",
    "app/package.json",
    "app/package-lock.json",
    "app/vite.config.ts",
    "app/tsconfig.json",
    "app/index.html",
];

fn main() {
    for input in SOURCE_INPUTS {
        println!("cargo:rerun-if-changed={input}");
    }

    let dist_path = Path::new("app/dist");
    let should_build = !dist_path.exists()
        || std::env::var("FORCE_REBUILD").is_ok()
        || frontend_is_stale(dist_path);

    if should_build {
        println!("cargo:warning=Building frontend assets...");

        // Install dependencies if node_modules doesn't exist
        let node_modules = Path::new("app/node_modules");
        if !node_modules.exists() {
            println!("cargo:warning=Installing npm dependencies...");
            let npm_install = Command::new("npm")
                .args(["ci"])
                .current_dir("app")
                .status()
                .expect("Failed to run npm ci");

            if !npm_install.success() {
                panic!("npm ci failed");
            }
        }

        // Build the frontend
        let npm_build = Command::new("npm")
            .args(["run", "build"])
            .current_dir("app")
            .status()
            .expect("Failed to run npm build");

        if !npm_build.success() {
            panic!("npm build failed");
        }

        println!("cargo:warning=Frontend build completed");
    }
}

/// True when any tracked frontend source is newer than the built `dist/`, so a
/// stale `dist` (left over from an earlier build) is rebuilt instead of being
/// silently re-embedded.
fn frontend_is_stale(dist_path: &Path) -> bool {
    let Some(dist_mtime) = newest_mtime(dist_path) else {
        return true; // dist empty or unreadable -> rebuild
    };
    SOURCE_INPUTS
        .iter()
        .filter_map(|src| newest_mtime(Path::new(src)))
        .any(|src_mtime| src_mtime > dist_mtime)
}

/// Newest modification time at or under `path`, recursing into directories.
fn newest_mtime(path: &Path) -> Option<SystemTime> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_file() {
        return meta.modified().ok();
    }
    if !meta.is_dir() {
        return None;
    }
    let mut newest: Option<SystemTime> = None;
    for entry in std::fs::read_dir(path).ok()?.flatten() {
        if let Some(mtime) = newest_mtime(&entry.path()) {
            newest = Some(newest.map_or(mtime, |cur| cur.max(mtime)));
        }
    }
    newest
}
