# Queens Game

A full-stack Rust implementation of a 9x9 Queens logic game.

The app uses:

- Axum for the Rust HTTP server and JSON API.
- Dioxus 0.7.9 with SSR for Rust-rendered page shells and a Rust/WASM client.
- A shared Rust crate for puzzle data, board cell helpers, auto-mark rules, and validation.
- Embedded Rust routes for static CSS and SVG assets, plus a generated WASM client bundle.
- A bundled set of 9x9 puzzle region layouts.

## Develop

```sh
nix develop path:$PWD
./scripts/build_client.sh
cargo run
```

Then open `http://127.0.0.1:3000`.

