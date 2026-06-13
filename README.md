# Queens Game

A full-stack Rust implementation of a 9x9 Queens logic game.

The app uses:

- Axum for the Rust HTTP server and JSON API.
- Dioxus 0.7.9 with SSR for Rust-rendered UI.
- Embedded Rust routes for static CSS, JS, and SVG assets.
- A bundled set of 9x9 puzzle region layouts.

## Develop

```sh
nix develop path:$PWD
cargo run
```

Then open `http://127.0.0.1:3000`.

