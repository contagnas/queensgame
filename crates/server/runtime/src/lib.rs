use std::{
    env, fs,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
};

pub fn bind_addr() -> SocketAddr {
    if let Ok(addr) = std::env::var("QUEENSGAME_ADDR") {
        return addr.parse().expect(
            "QUEENSGAME_ADDR must be a valid socket address, like 127.0.0.1:3000 or 0.0.0.0:3000",
        );
    }

    if let Ok(port) = std::env::var("PORT") {
        return format!("0.0.0.0:{port}")
            .parse()
            .expect("PORT must be a valid TCP port");
    }

    "127.0.0.1:3000"
        .parse()
        .expect("default bind address must be valid")
}

pub fn client_dist_dir() -> PathBuf {
    env::var_os("QUEENSGAME_CLIENT_DIST")
        .map(PathBuf::from)
        .or_else(bazel_client_dist_dir)
        .unwrap_or_else(|| PathBuf::from("dist/client"))
}

fn bazel_client_dist_dir() -> Option<PathBuf> {
    const CLIENT_JS_RUNFILE: &str =
        "crates/client/src/queensgame_client_bindgen/queensgame_client.js";

    for runfiles_dir in bazel_runfiles_dirs() {
        for workspace in ["_main", "queensgame"] {
            let client_js = runfiles_dir.join(workspace).join(CLIENT_JS_RUNFILE);
            if client_js.is_file() {
                return client_js.parent().map(FsPath::to_path_buf);
            }
        }
    }

    env::var_os("RUNFILES_MANIFEST_FILE")
        .and_then(|manifest| bazel_manifest_runfile(FsPath::new(&manifest), CLIENT_JS_RUNFILE))
        .and_then(|client_js| client_js.parent().map(FsPath::to_path_buf))
}

fn bazel_runfiles_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(dir) = env::var_os("RUNFILES_DIR").map(PathBuf::from) {
        dirs.push(dir);
    }

    if let Ok(current_exe) = env::current_exe() {
        dirs.push(PathBuf::from(format!("{}.runfiles", current_exe.display())));
    }

    if let Some(arg0) = env::args_os().next() {
        dirs.push(PathBuf::from(format!(
            "{}.runfiles",
            FsPath::new(&arg0).display()
        )));
    }

    dirs
}

fn bazel_manifest_runfile(manifest: &FsPath, runfile_suffix: &str) -> Option<PathBuf> {
    let manifest = fs::read_to_string(manifest).ok()?;
    manifest.lines().find_map(|line| {
        let (logical_path, real_path) = line.split_once(' ').unwrap_or((line, line));
        logical_path
            .ends_with(runfile_suffix)
            .then(|| PathBuf::from(real_path))
    })
}
