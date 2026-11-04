# Sandbox environments derived from gungnir.toml and languages/<id>/language.toml.
#
#   $out/base/{bin,closure}             [sandbox].packages
#   $out/measure/{bin,closure}          [measure].packages
#   $out/language/<id>/{bin,closure}    every languages/<id>/language.toml
#   $out/harness/<name>/{bin,closure}   [harnesses.<name>].package
#
# `bin` links to the bin directory of a buildEnv and `closure` to the closureInfo store-path list
# of that env. The TOML files are read at evaluation time, so no output references the flake
# source, whose copy in the store holds the hidden tests.
{
  pkgs,
  lib,
  root,
}:
let
  catalog = lib.importTOML (root + "/gungnir.toml");

  languageIds = lib.attrNames (
    lib.filterAttrs (_: type: type == "directory") (builtins.readDir (root + "/languages"))
  );

  # Packages nixpkgs lacks, named in language.toml like any nixpkgs attribute.
  local = {
    typescript-bun = import ./typescript-bun.nix { inherit pkgs lib; };
  };

  package = attrPath: lib.getAttrFromPath (lib.splitString "." attrPath) (pkgs // local);

  environment = dir: packages: {
    inherit dir;
    env = pkgs.buildEnv {
      name = "gungnir-${lib.replaceStrings [ "/" ] [ "-" ] dir}";
      paths = map package packages;
    };
  };

  environments = [
    (environment "base" catalog.sandbox.packages)
    (environment "measure" catalog.measure.packages)
  ]
  ++ map (
    id: environment "language/${id}" (lib.importTOML (root + "/languages/${id}/language.toml")).packages
  ) languageIds
  ++ lib.mapAttrsToList (name: harness: environment "harness/${name}" [ harness.package ]) (
    catalog.harnesses
  );

  link =
    { dir, env }:
    ''
      mkdir -p $out/${dir}
      ln -s ${env}/bin $out/${dir}/bin
      ln -s ${pkgs.closureInfo { rootPaths = [ env ]; }}/store-paths $out/${dir}/closure
    '';
in
pkgs.runCommand "gungnir-toolchains" { } (lib.concatMapStrings link environments)
