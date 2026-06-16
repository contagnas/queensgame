# Boardmage

A full-stack Rust implementation of 9x9 Queens and Minesweeper web games.

The app uses:

- Axum for the Rust HTTP server and JSON API.
- Dioxus 0.7.9 with SSR for Rust-rendered page shells and a Rust/WASM client.
- A shared Rust crate for puzzle data, board helpers, Queens validation, and Minesweeper rules.
- Embedded Rust routes for static CSS and SVG assets, plus a generated WASM client bundle.
- Bazel with `rules_rust` for hermetic Rust server, shared crate, and WASM client builds.
- A bundled set of 9x9 puzzle region layouts.
- A classic Expert Minesweeper board at `/minesweeper`.

## Develop

```sh
nix develop path:$PWD
./scripts/build_client.sh
cargo run
```

Then open `http://127.0.0.1:3000`.

## Bazel

The Bazel build uses Bzlmod, `rules_rust`, `crate_universe`, and
`rules_rust_wasm_bindgen`, plus `rules_img` for production container images.
Bazelisk is included in the Nix dev shell and reads `.bazelversion`. Rust builds
are pinned to rustc 1.96.0 and Rust edition 2024. `.bazelrc` optionally imports
an ignored `user.bazelrc`; put
`build --config=nix` there to make the Nix Bazel settings the local default.

On the first run after dependency changes, repin the generated crate universe:

```sh
CARGO_BAZEL_REPIN=1 bazelisk sync --only=crates
```

Build and test the Rust targets:

```sh
bazelisk build //:server //:client
bazelisk test //crates/shared/src:unit_test //crates/server/src:unit_test
```

Run clippy through the `rules_rust` clippy aspect:

```sh
bazelisk build --config=clippy //crates/shared/src:queensgame_shared //crates/server/src:queensgame
bazelisk build --config=wasm --config=clippy //crates/client/src:queensgame_client_wasm
```

Run rustfmt through the `rules_rust` rustfmt aspect:

```sh
bazelisk build --config=rustfmt //crates/shared/src:queensgame_shared //crates/server/src:queensgame
bazelisk build --config=wasm --config=rustfmt //crates/client/src:queensgame_client_wasm
```

Generate a `rust-project.json` file for rust-analyzer:

```sh
bazelisk run @rules_rust//tools/rust_analyzer:gen_rust_project
```

Run the full app with the Bazel-built WASM bundle:

```sh
bazelisk run //:server
```

Build the OCI image:

```sh
bazelisk build //crates/server/src:image
```

Load it into Docker or containerd:

```sh
bazelisk run //crates/server/src:image.load
```

Build a Docker-compatible tarball instead:

```sh
bazelisk build //crates/server/src:image.load --output_groups=tarball
```

To host on your LAN, bind to all interfaces:

```sh
QUEENSGAME_ADDR=0.0.0.0:3000 cargo run
```

Then open `http://<your-lan-ip>:3000`, such as `http://192.168.0.105:3000`. On NixOS, make sure TCP port 3000 is allowed through the firewall.
