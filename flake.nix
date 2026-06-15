{
  description = "Full-stack Rust Queens web game";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              curl
              jq
              lld
              perl
              railway
              rustc
              rustfmt
              clippy
              wasm-bindgen-cli
            ];

            RUST_BACKTRACE = "1";
          };
        });

      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "queensgame";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = with pkgs; [
              lld
              makeWrapper
              wasm-bindgen-cli
            ];
            cargoBuildFlags = [ "-p" "queensgame" ];
            cargoTestFlags = [ "-p" "queensgame" "-p" "queensgame-shared" ];
            preBuild = ''
              cargo build --release -p queensgame-client --target wasm32-unknown-unknown
              mkdir -p dist/client
              wasm-bindgen \
                --target web \
                --out-dir dist/client \
                --out-name queensgame_client \
                target/wasm32-unknown-unknown/release/queensgame_client.wasm
            '';
            postInstall = ''
              mkdir -p $out/share/queensgame/client
              cp -R dist/client/. $out/share/queensgame/client/
              wrapProgram $out/bin/queensgame \
                --set QUEENSGAME_CLIENT_DIST $out/share/queensgame/client
            '';
          };
        });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/queensgame";
        };
      });
    };
}
