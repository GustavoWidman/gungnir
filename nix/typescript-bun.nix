# tsc with the type declarations of Bun's runtime, so a TypeScript sandbox type-checks a
# program offline against the APIs Bun runs it with, and no workspace needs a node_modules.
#
# The wrapper passes --typeRoots, so a tsconfig that lists "types": ["bun"] resolves @types/bun
# from the store. The declarations track nixpkgs' bun: bun-types and @types/bun at Bun's
# version, @types/node at the Node.js version Bun reports as process.version, and the
# undici-types that @types/node pins. Only the .d.ts files and package.json manifests are kept.
{ pkgs, lib }:
let
  bun = "1.4.2";

  npm =
    name: version: hash:
    pkgs.fetchurl {
      url = "https://registry.npmjs.org/${name}/-/${baseNameOf name}-${version}.tgz";
      inherit hash;
    };

  packages = {
    "@types/bun" =
      npm "@types/bun" bun
        "sha512-GimotNn7+ZV0uVArItBbriZsR1oNf0+WTzPkdcFrzShI7k2norL0uzEaJT8T33dWr7O/c9ZDuAFQrctKCi72oQ==";
    "bun-types" =
      npm "bun-types" bun
        "sha512-bxV1FgK7yBIzjRe5zBozIM4Bem11ZJcCXSrjWRG3YWLt8yFDePu4cLjpebO8OvPeIE9trbyPF4fuj3Cia4Fj3w==";
    "@types/node" =
      npm "@types/node" "26.3.0"
        "sha512-L3fgrnchriRC2ExBflb8j4uZZURHZfQsmQeyVzhjcHW4kkwVyo8/0h1B2MVzMTrYUJYu6G7EWs14hW/L9putqw==";
    "undici-types" =
      npm "undici-types" "8.3.0"
        "sha512-j375ScV60dom+YkPFIfTLcOiPxkN/buHz5GobjLhixFuANaNs3C9l4GmrWqejgXWJ7BbJcFYpTEUkS1Ge8bpZQ==";
  };

  declarations = pkgs.runCommand "bun-types-${bun}" { } ''
    ${lib.concatStrings (
      lib.mapAttrsToList (name: tarball: ''
        mkdir -p $out/node_modules/${name}
        tar -xzf ${tarball} -C $out/node_modules/${name} --strip-components=1
      '') packages
    )}
    find $out -type f ! -name '*.d.ts' ! -name package.json -delete
    find $out -type d -empty -delete
  '';
in
assert lib.assertMsg (pkgs.bun.version == bun)
  "nix/typescript-bun.nix declares Bun ${bun}, but nixpkgs has bun ${pkgs.bun.version}; update the declarations to match";
pkgs.runCommand "typescript-bun-${pkgs.typescript.version}"
  { nativeBuildInputs = [ pkgs.makeBinaryWrapper ]; }
  ''
    makeBinaryWrapper ${lib.getExe' pkgs.typescript "tsc"} $out/bin/tsc \
      --add-flags "--typeRoots ${declarations}/node_modules/@types"
  ''
