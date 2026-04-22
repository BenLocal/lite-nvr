use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app/src");
    println!("cargo:rerun-if-changed=app/package.json");
    println!("cargo:rerun-if-changed=app/package-lock.json");
    println!("cargo:rerun-if-changed=app/vite.config.ts");
    println!("cargo:rerun-if-changed=app/tsconfig.json");
    println!("cargo:rerun-if-changed=app/index.html");

    // Check if dist directory exists and is up to date
    let dist_path = std::path::Path::new("app/dist");
    let should_build = !dist_path.exists() || std::env::var("FORCE_REBUILD").is_ok();

    if should_build {
        println!("cargo:warning=Building frontend assets...");

        // Install dependencies if node_modules doesn't exist
        let node_modules = std::path::Path::new("app/node_modules");
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
