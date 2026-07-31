{
  description = "Graduate Rust CLI/TUI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        workspace = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        graduate = pkgs.rustPlatform.buildRustPackage {
          pname = "graduate";
          version = workspace.workspace.package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "--package" "graduate-cli" ];
          cargoTestFlags = [ "--workspace" ];
          meta = with pkgs.lib; {
            description = "Graduate, a Jira Cloud command-line and terminal client";
            homepage = "https://github.com/treramey/graduate";
            license = licenses.mit;
            mainProgram = "gd";
          };
        };
      in {
        packages.default = graduate;
        packages.graduate = graduate;
        apps.default = flake-utils.lib.mkApp { drv = graduate; };
        devShells.default = pkgs.mkShell { inputsFrom = [ graduate ]; packages = with pkgs; [ cargo clippy rustc rustfmt ]; };
      });
}
