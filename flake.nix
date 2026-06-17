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
          gitExe = lib.getExe pkgs.git;
          grepExe = lib.getExe pkgs.gnugrep;
        in
        {
          default = pkgs.mkShell {
            packages = [
              bazelAlias
            ];

            RUST_BACKTRACE = "1";

            shellHook = ''
              workspace_dir=$(${gitExe} rev-parse --show-toplevel 2>/dev/null || true)
              if [ -n "$workspace_dir" ]; then
                user_bazelrc="$workspace_dir/user.bazelrc"
                if ! { [ -f "$user_bazelrc" ] && ${grepExe} -q -- "^build --config=nix$" "$user_bazelrc"; }; then
                  user_bazelrc_update="# Added by nix develop for NixOS Bazel actions.
build --config=nix"
                  echo "nix develop: adding 'build --config=nix' to $user_bazelrc"
                  printf "%s\n" "$user_bazelrc_update" >> "$user_bazelrc"
                fi
              fi
            '';
          };
        });
    };
}
