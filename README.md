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

To host on your LAN, bind to all interfaces:

```sh
QUEENSGAME_ADDR=0.0.0.0:3000 cargo run
```

Then open `http://<your-lan-ip>:3000`, such as `http://192.168.0.105:3000`. On NixOS, make sure TCP port 3000 is allowed through the firewall.

