{
  description = "jphfmt — a zero-config, opinionated C formatter: one uniform bracket rule, no column alignment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };

    camas = {
      url = "github:JPHutchins/camas/0.1.25";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      flake-utils,
      rust-overlay,
      advisory-db,
      camas,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        inherit (pkgs) lib;

        rustStable = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };

        msrv = (lib.importTOML ./Cargo.toml).package.rust-version;
        rustMSRV = pkgs.rust-bin.stable."${msrv}.0".default;

        craneLib = (crane.mkLib pkgs).overrideToolchain rustStable;
        craneLibMSRV = (crane.mkLib pkgs).overrideToolchain rustMSRV;

        # Keep tests/: the conformance suite reads .c fixtures (include_str! and a
        # runtime tests/cases/ walk) that cleanCargoSource would drop.
        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            (craneLib.fileset.commonCargoSources ./.)
            ./tests
          ];
        };

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];
        };

        testArgs = commonArgs // {
          cargoExtraArgs = "--all-features";
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        nodejs = pkgs.nodejs_22;

        # editors/vscode's node_modules, built hermetically from package-lock.json
        # (no `npm ci`): importNpmLock reads the lockfile's integrity hashes and
        # fetches each tarball as a fixed-output derivation.
        vscodeNodeModules = pkgs.importNpmLock.buildNodeModules {
          npmRoot = ./editors/vscode;
          inherit nodejs;
        };

        jphfmt = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            doCheck = false;
            meta.mainProgram = "jphfmt";
          }
        );

        checks = {
          test = craneLib.cargoNextest (
            testArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
              cargoNextestPartitionsExtraArgs = "--no-tests=pass";
            }
          );

          test-msrv = craneLibMSRV.cargoNextest (
            testArgs
            // {
              cargoArtifacts = craneLibMSRV.buildDepsOnly testArgs;
              partitions = 1;
              partitionType = "count";
              cargoNextestPartitionsExtraArgs = "--no-tests=pass";
            }
          );

          lint = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets --all-features -- -D warnings";
            }
          );

          fmt = craneLib.cargoFmt { inherit src; };

          doc = craneLib.cargoDoc (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoDocExtraArgs = "--no-deps --all-features";
              env.RUSTDOCFLAGS = "-D warnings";
            }
          );

          audit = craneLib.cargoAudit { inherit src advisory-db; };

          fmt-nix = pkgs.runCommand "nixfmt-check" { nativeBuildInputs = [ pkgs.nixfmt ]; } ''
            nixfmt --check ${./flake.nix}
            touch $out
          '';
        };

        mkCheckApp =
          name: drv:
          flake-utils.lib.mkApp {
            drv = pkgs.writeShellApplication {
              name = "jphfmt-${name}";
              text = ''
                echo "jphfmt ${name}: ok (${drv})"
              '';
            };
          };
      in
      {
        packages = {
          default = jphfmt;
          inherit jphfmt;
        };

        apps = {
          default = flake-utils.lib.mkApp { drv = jphfmt; };

          test = mkCheckApp "test" checks.test;
          test-msrv = mkCheckApp "test-msrv" checks.test-msrv;
          lint = mkCheckApp "lint" checks.lint;
          fmt = mkCheckApp "fmt" checks.fmt;
          doc = mkCheckApp "doc" checks.doc;
          audit = mkCheckApp "audit" checks.audit;
          fmt-nix = mkCheckApp "fmt-nix" checks.fmt-nix;

          fix = flake-utils.lib.mkApp {
            drv = pkgs.writeShellApplication {
              name = "jphfmt-fix";
              runtimeInputs = [ rustStable ];
              text = ''
                cargo fmt --all
                cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features
              '';
            };
          };
        };

        devShells.default = craneLib.devShell {
          packages = [
            camas.packages.${system}.default
            nodejs
            pkgs.uv
            pkgs.cargo-audit
            pkgs.cargo-mutants
            pkgs.cargo-nextest
            pkgs.nixfmt
          ];
          shellHook = ''
            rm -rf editors/vscode/node_modules
            ln -s ${vscodeNodeModules}/node_modules editors/vscode/node_modules
          '';
        };

        formatter = pkgs.nixfmt;
      }
    );
}
