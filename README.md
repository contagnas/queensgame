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
bazel run //:server
```

Then open `http://127.0.0.1:3000`.

## Bazel

The Bazel build uses Bzlmod, `rules_rust`, `crate_universe`, and
`rules_rust_wasm_bindgen`, plus `rules_img` for production container images.
Bazelisk is included in the Nix dev shell and reads `.bazelversion`. Inside the
Nix dev shell, use `bazel`; that wrapper delegates to Bazelisk. The shell also
creates or updates the ignored workspace `user.bazelrc` with `build --config=nix`
so fresh checkouts work on NixOS, including direct `bazelisk` usage inside the
shell. Rust builds are pinned to rustc 1.96.0 and Rust edition 2024.

On the first run after dependency changes, repin the generated crate universe:

```sh
CARGO_BAZEL_REPIN=1 bazel fetch //...
```

The external Rust dependency specs and lockfile live under `third_party/rust`;
application crates are built from Bazel targets rather than package manifests.

Build and test the Rust targets:

```sh
bazel build //:server //:client
bazel test //crates/shared:unit_test //crates/server:unit_test
```

Run strict Rust lint checks:

```sh
bazel run //:format.check
bazel build --config=clippy //...
bazel query 'kind("rust_(library|binary|shared_library) rule", //crates/client/... + //crates/shared/...)' \
  | xargs bazel build --config=wasm --config=clippy
```

The rules_lint format target checks workspace Rust and CSS files with
Bazel-managed formatter tools. Run `bazel run //:format` to rewrite files in
place. The clippy config enables the rules_rust clippy aspect on the requested
build targets and denies warnings plus `clippy::all`, `clippy::pedantic`, and
`clippy::nursery`. For a smaller target set, pass the same config to the target
you want to check:

```sh
bazel build --config=clippy //crates/server:queensgame
bazel build --config=wasm --config=clippy //crates/client:queensgame_client_wasm
```

Generate a `rust-project.json` file for rust-analyzer:

```sh
bazel run @rules_rust//tools/rust_analyzer:gen_rust_project
```

Run the full app with the Bazel-built WASM bundle:

```sh
bazel run //:server
```

Build the OCI image:

```sh
bazel build //crates/server:image
```

Load it into Docker or containerd:

```sh
bazel run //crates/server:image.load
```

Build a Docker-compatible tarball instead:

```sh
bazel build //crates/server:image.load --output_groups=tarball
```

To host on your LAN, bind to all interfaces:

```sh
QUEENSGAME_ADDR=0.0.0.0:3000 bazel run //:server
```

Then open `http://<your-lan-ip>:3000`, such as `http://192.168.0.105:3000`. On NixOS, make sure TCP port 3000 is allowed through the firewall.
