# Queens Game

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
`rules_rust_wasm_bindgen`. Bazelisk is included in the Nix dev shell and reads
`.bazelversion`.

On the first run after dependency changes, repin the generated crate universe:

```sh
CARGO_BAZEL_REPIN=1 bazelisk mod deps
```

Build and test the Rust targets:

```sh
bazelisk build --config=nix //:server //:client
bazelisk test --config=nix //crates/shared:unit_test //crates/server:unit_test
```

Run the full app with the Bazel-built WASM bundle:

```sh
bazelisk run --config=nix //:server
```

To host on your LAN, bind to all interfaces:

```sh
QUEENSGAME_ADDR=0.0.0.0:3000 cargo run
```

Then open `http://<your-lan-ip>:3000`, such as `http://192.168.0.105:3000`. On NixOS, make sure TCP port 3000 is allowed through the firewall.
