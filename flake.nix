{
  description = "gungnir: a benchmark of coding agents across programming languages and reasoning effort";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    flake-parts.url = "github:hercules-ci/flake-parts";
    systems.url = "github:nix-systems/default-linux";
    devshell.url = "github:numtide/devshell";
  };

  outputs =
    inputs@{
      flake-parts,
      fenix,
      systems,
      devshell,
      nixpkgs,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = import systems;
      imports = [ devshell.flakeModule ];

      perSystem =
        { system, lib, ... }:
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfreePredicate = pkg: lib.getName pkg == "claude-code";
          };

          rust = fenix.packages.${system}.fromToolchainFile {
            file = ./rust-toolchain.toml;
            sha256 = "sha256-2wqUa1sFNg8sdadO5YXcUSwpoBek+e+viCiSqnpUbdM=";
          };
        in
        {
          _module.args.pkgs = pkgs;

          devshells.default = {
            packages = [
              rust
              pkgs.stdenv.cc
              pkgs.bubblewrap
              pkgs.hyperfine
              pkgs.python3Packages.lizard
              pkgs.jq
              pkgs.typst
              pkgs.claude-code
              pkgs.codex
            ];
            env = [
              {
                name = "TYPST_FONT_PATHS";
                value = "${pkgs.gyre-fonts}/share/fonts";
              }
            ];
            motd = "";
          };
        };
    };
}
