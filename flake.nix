{
  description = "External regression tests for herdr Nexus";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-build = {
      url = "github:LiGoldragon/rust-build";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    herdr-nexus = {
      url = "github:LiGoldragon/herdr-nexus/9047730";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-build, herdr-nexus }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        rust = rust-build.lib.${system}.fromPkgs pkgs;
        prepared = pkgs.runCommand "herdr-nexus-external-test-source" {
          nativeBuildInputs = [ pkgs.patch ];
        } ''
          cp -R ${herdr-nexus} "$out"
          chmod -R u+w "$out"
          patch --fuzz=0 --directory="$out" --strip=1 < ${./patches/external-tests.patch}
          mkdir -p "$out/external-tests"
          cp ${./tests/daemon.rs} "$out/external-tests/daemon.rs"
        '';
        source = rust.cleanSource {
          root = prepared;
          extraFilters = [
            (path: type: type == "regular" && pkgs.lib.hasSuffix ".ethos" (toString path))
          ];
        };
        cargoArtifacts = rust.craneLib.buildDepsOnly {
          src = source;
          cargoExtraArgs = "--workspace --all-features";
        };
      in {
        checks.regressions = rust.craneLib.cargoTest {
          pname = "herdr-nexus-external-regressions";
          version = "0.1.0";
          src = source;
          inherit cargoArtifacts;
          cargoTestExtraArgs = "-p herdr-nexus-daemon --lib --all-features";
        };
      });
}
