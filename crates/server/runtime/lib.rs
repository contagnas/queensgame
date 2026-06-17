use std::{
    env, fs,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
};

const CLIENT_JS: &str = "queensgame_client.js";
const CLIENT_DIST_RUNFILE_DIRS: &[&str] = &[
    "crates/client/queensgame_client_bindgen_optimized",
    "crates/client/queensgame_client_bindgen",
];

/// Returns the socket address the HTTP server should bind to.
///
/// # Panics
///
/// Panics when `QUEENSGAME_ADDR` or `PORT` is set to an invalid socket address.
#[must_use]
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

/// Returns the Bazel-built client asset directory.
///
/// # Panics
///
/// Panics when neither `QUEENSGAME_CLIENT_DIST` nor Bazel runfiles point to the client bundle.
#[must_use]
pub fn client_dist_dir() -> PathBuf {
    env::var_os("QUEENSGAME_CLIENT_DIST")
        .map(PathBuf::from)
        .or_else(bazel_client_dist_dir)
        .expect("could not find Bazel-built client assets; set QUEENSGAME_CLIENT_DIST")
}

fn bazel_client_dist_dir() -> Option<PathBuf> {
    for runfiles_dir in bazel_runfiles_dirs() {
        for workspace in ["_main", "queensgame"] {
            for client_dist_runfile_dir in CLIENT_DIST_RUNFILE_DIRS {
                let client_dist = runfiles_dir.join(workspace).join(client_dist_runfile_dir);
                if client_dist.join(CLIENT_JS).is_file() {
                    return Some(client_dist);
                }
            }
        }
    }

    env::var_os("RUNFILES_MANIFEST_FILE")
        .and_then(|manifest| bazel_manifest_client_dist_dir(FsPath::new(&manifest)))
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

fn bazel_manifest_client_dist_dir(manifest: &FsPath) -> Option<PathBuf> {
    let manifest = fs::read_to_string(manifest).ok()?;
    manifest.lines().find_map(|line| {
        let (logical_path, real_path) = line.split_once(' ').unwrap_or((line, line));

        for client_dist_runfile_dir in CLIENT_DIST_RUNFILE_DIRS {
            if logical_path.ends_with(client_dist_runfile_dir) {
                let client_dist = PathBuf::from(real_path);
                if client_dist.join(CLIENT_JS).is_file() {
                    return Some(client_dist);
                }
            }

            let client_js_runfile = format!("{client_dist_runfile_dir}/{CLIENT_JS}");
            if logical_path.ends_with(&client_js_runfile) {
                return PathBuf::from(real_path).parent().map(FsPath::to_path_buf);
            }
        }

        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn manifest_resolver_finds_optimized_tree_artifact() {
        let temp_dir = temp_test_dir("optimized");
        let client_dist = temp_dir.join("client_dist");
        fs::create_dir_all(&client_dist).unwrap();
        fs::write(client_dist.join(CLIENT_JS), "").unwrap();

        let manifest = temp_dir.join("MANIFEST");
        fs::write(
            &manifest,
            format!(
                "_main/crates/client/queensgame_client_bindgen_optimized {}\n",
                client_dist.display()
            ),
        )
        .unwrap();

        assert_eq!(bazel_manifest_client_dist_dir(&manifest), Some(client_dist));

        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn manifest_resolver_finds_plain_client_js_entry() {
        let temp_dir = temp_test_dir("plain");
        let client_dist = temp_dir.join("client_dist");
        fs::create_dir_all(&client_dist).unwrap();
        let client_js = client_dist.join(CLIENT_JS);
        fs::write(&client_js, "").unwrap();

        let manifest = temp_dir.join("MANIFEST");
        fs::write(
            &manifest,
            format!(
                "_main/crates/client/queensgame_client_bindgen/{CLIENT_JS} {}\n",
                client_js.display()
            ),
        )
        .unwrap();

        assert_eq!(bazel_manifest_client_dist_dir(&manifest), Some(client_dist));

        fs::remove_dir_all(temp_dir).unwrap();
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("queensgame-runtime-{name}-{nanos}"))
    }
}
