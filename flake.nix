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
        grad = pkgs.rustPlatform.buildRustPackage {
          pname = "grad";
          version = workspace.workspace.package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "--package" "graduation-cli" ];
          cargoTestFlags = [ "--workspace" ];
          meta = with pkgs.lib; {
            description = "Graduate, a Jira Cloud command-line and terminal client";
            homepage = "https://github.com/treramey/graduate";
            license = licenses.mit;
            mainProgram = "gd";
          };
        };
      in {
        packages.default = grad;
        packages.grad = grad;
        apps.default = flake-utils.lib.mkApp { drv = grad; };
        devShells.default = pkgs.mkShell { inputsFrom = [ grad ]; packages = with pkgs; [ cargo clippy rustc rustfmt ]; };
      });
}
