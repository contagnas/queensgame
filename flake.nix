{
  description = "Boardmage development shell";

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
          inherit (pkgs) lib;
          bazelAlias = pkgs.writeShellScriptBin "bazel" ''
            exec ${lib.getExe pkgs.bazelisk} "$@"
          '';
        in
        {
          default = pkgs.mkShell {
            packages = [
              bazelAlias
            ] ++ (with pkgs; [
              bazelisk
              buildifier
              patchelf
            ]);

            RUST_BACKTRACE = "1";
          };
        });
    };
}
