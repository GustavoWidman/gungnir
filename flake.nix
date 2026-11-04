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
      crane,
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
            overlays = [
              (_: _: {
                # The newest nightly in the fenix lock, with rustc, cargo and rust-std, for the
                # benchmarked rust language.
                rust-nightly = fenix.packages.${system}.minimal.toolchain;
              })
            ];
          };

          rust = fenix.packages.${system}.fromToolchainFile {
            file = ./rust-toolchain.toml;
            sha256 = "sha256-2wqUa1sFNg8sdadO5YXcUSwpoBek+e+viCiSqnpUbdM=";
          };

          craneLib = (crane.mkLib pkgs).overrideToolchain rust;

          args = {
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                (craneLib.fileset.commonCargoSources ./.)
                ./gungnir.toml
                ./languages
              ];
            };
            strictDeps = true;
          };

          cargoArtifacts = craneLib.buildDepsOnly args;

          gungnir = craneLib.buildPackage (args // { inherit cargoArtifacts; });

          toolchains = import ./nix/toolchains.nix {
            inherit pkgs lib;
            root = ./.;
          };

          wrapped = pkgs.symlinkJoin {
            name = "gungnir-${gungnir.version}";
            paths = [ gungnir ];
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postBuild = ''
              wrapProgram $out/bin/gungnir \
                --set GUNGNIR_TOOLCHAINS ${toolchains} \
                --prefix PATH : ${
                  lib.makeBinPath [
                    pkgs.bubblewrap
                    pkgs.util-linux
                    pkgs.python3Packages.lizard
                  ]
                }
            '';
            meta.mainProgram = "gungnir";
          };
        in
        {
          _module.args.pkgs = pkgs;

          packages.default = wrapped;
          packages.gungnir = wrapped;
          packages.toolchains = toolchains;

          checks = {
            package = gungnir;
            clippy = craneLib.cargoClippy (
              args
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--all-targets -- --deny warnings";
              }
            );
            fmt = craneLib.cargoFmt { inherit (args) src; };
          };

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
                name = "GUNGNIR_TOOLCHAINS";
                value = "${toolchains}";
              }
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
