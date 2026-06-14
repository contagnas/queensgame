# Development Notes

These notes capture current decisions and tradeoffs around the frontend framework,
websocket model, Bazel compatibility, and development reload options.

## Dioxus Fit

Dioxus is a reasonable fit for the current app, especially because this project is
intentionally full-stack Rust and benefits from shared Rust types between the
client and server.

The strongest part of the current architecture is the `crates/shared` crate. Room
messages, replay payloads, board validation, puzzle data types, and multiplayer
state can be reused by both the browser client and the Axum server. That reduces
client/server drift and keeps server-side verification close to the client model.

The weaker part is browser ergonomics. This game has a lot of web-native behavior:
pointer events, drag modes, local storage, websocket streaming, replay scrubbers,
mouse playback, timers, and responsive layout. Dioxus can do all of this, but the
Rust/WASM/browser boundary is more noticeable than it would be in a TypeScript UI.

Practical read:

- Dioxus is good enough for the current local/LAN full-stack Rust game.
- TypeScript plus a Rust backend would probably be faster for heavy UI iteration.
- A rewrite is not justified right now because the Dioxus app is working and the
  shared Rust boundary is valuable.

## Leptos Comparison

If starting over while staying Rust-only in the browser, Leptos might be a slightly
better web-first framework choice. Its fine-grained reactivity and web-focused
SSR/hydration model are a strong fit for browser applications.

Dioxus still has a stronger built-in full-stack websocket story. Dioxus Fullstack
documents websocket support with shared server/client types, typed inputs and
outputs, reactive UI wrappers, and Axum underneath. Leptos can absolutely use
websockets, but the websocket layer is less central to its core model and is more
likely to be wired manually with Axum, web-sys, gloo, or community crates.

For this app:

- Leptos may be cleaner for pure Rust web UI reactivity.
- Dioxus is defensible because multiplayer websocket state is central.
- TypeScript would still be the smoother choice for the most polished browser UI.

## Current Dioxus Usage

This repo does not currently use the Dioxus CLI.

There is no `dx serve`, `dx build`, or `Dioxus.toml` dependency in the workflow.
The current client build is plain Cargo plus wasm-bindgen:

```sh
cargo build --release -p queensgame-client --target wasm32-unknown-unknown
wasm-bindgen \
  --target web \
  --out-dir dist/client \
  --out-name queensgame_client \
  target/wasm32-unknown-unknown/release/queensgame_client.wasm
```

The server is a normal Axum binary. It serves the generated client assets from
`QUEENSGAME_CLIENT_DIST` or `dist/client`, and uses `dioxus-ssr` only to render
the initial HTML shells.

This keeps the build explicit and makes Nix/Bazel integration easier than a
project that depends on Dioxus CLI bundling.

## Websockets

The current websocket layer is custom Axum, not Dioxus Fullstack websockets.

The room endpoint lives on the server, accepts JSON messages over an Axum
websocket, deserializes them as shared Rust enums, mutates server-owned room
state, and broadcasts serialized shared snapshot/update messages back to clients.

This means we already get the most important property: shared typed protocol data.
We do not currently get Dioxus's higher-level websocket wrappers or full-stack
server function integration.

Keeping the custom Axum websocket layer is reasonable because room state,
recording streams, live replay frames, and optional mouse recordings are specific
to this game and already work as an explicit protocol.

## Bazel And rules_rust

Moving the build to Bazel with `rules_rust` should be feasible. Dioxus itself is
not the hard part because the current app builds as ordinary Rust crates.

The Bazel shape would likely be:

- `rust_library` for `queensgame-shared`.
- `rust_binary` for the Axum server.
- A wasm32 Rust target for `queensgame-client`.
- `rust_wasm_bindgen` to generate browser JS/WASM assets.
- A packaging or runfiles step so the server can locate the generated client
  assets.
- Declared inputs for embedded files such as puzzle JSON, CSS, and SVG assets.

The main integration cost would be Bazel-specific:

- setting up wasm32 target/platform transitions;
- configuring `crate_universe` dependency features correctly for Dioxus, web-sys,
  wasm-bindgen, and gloo;
- deciding whether static assets are embedded with `include_str!` or served from
  runfiles in dev;
- replacing the ad hoc `dist/client` convention with a Bazel output path or
  wrapper-provided `QUEENSGAME_CLIENT_DIST`.

Because the repo does not depend on the Dioxus CLI, Bazel does not need to emulate
`dx build` to produce the current app.

## Live Reload And Hot Reload

There are two different concepts:

- Live reload: rebuild/restart/re-serve, then refresh the browser.
- Hot reload: patch the running UI without a full rebuild or page refresh.

Dioxus CLI hot reload is stronger than ordinary live reload. With `dx serve`,
Dioxus can parse changed `rsx!` blocks and patch the running `VirtualDom` without
recompiling the whole app. Dioxus 0.7 also has experimental Rust hot-patching via
`dx serve --hotpatch`.

That does not come for free in this repo because we are not using `dx serve`.

With Bazel, the pragmatic path would be:

- implement full live reload with `ibazel`, `bazel watch`, or a small watcher;
- rebuild the server/client bundle on changes;
- send a websocket or SSE reload signal to the browser;
- call `location.reload()` in development;
- optionally add CSS/static asset reload without a full page refresh.

Implementing true Dioxus RSX hot reload under Bazel would be much harder and more
version-sensitive because it would mean integrating with Dioxus devtools/hot
reload protocol and its `VirtualDom` patching model. Rust hot-patching under
Bazel is likely not worth pursuing for this app, especially because important
logic lives across workspace crates.

Recommended dev workflow improvement before adopting Dioxus CLI:

```sh
watchexec -r -w crates -w static -w data \
  './scripts/build_client.sh && QUEENSGAME_ADDR=0.0.0.0:8080 cargo run'
```

If the project moves to Bazel, implement Bazel live reload first and only revisit
Dioxus hot reload if UI iteration speed becomes a real bottleneck.
