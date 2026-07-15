{ inputs, self, ... }:
{
  imports = [
    inputs.pre-commit-hooks-nix.flakeModule
    inputs.hercules-ci-effects.flakeModule
    inputs.nix-unit.modules.flake.default
  ];
  perSystem =
    {
      config,
      inputs',
      pkgs,
      ...
    }:
    {
      # Run with either:
      #   nix-unit --flake .#tests.systems.<system>
      # or, slower:
      #   nix build .#checks.<system>.nix-unit
      # Deliberately does *not* pass `adapter` or `fixture`: forcing
      # those packages from inside nix-unit-check triggers dream2nix's
      # `builtins.fetchGit` for the workspace's Cargo git deps, which
      # can't reach out from a network-restricted sandbox. The tests
      # instead read a checked-in schema snapshot (see the file for
      # rationale). `checks.fixture-schema-snapshot` keeps the
      # snapshot honest.
      nix-unit.tests.lib = import ../nix/tests/lib.nix {
        inherit (pkgs) lib;
        inherit self;
        nixops4 = inputs.nixops4;
      };

      # End-to-end proof that `nixops4 apply` works against a
      # tfproto-generated provider. Later commits extend this itest
      # with feature-specific scenarios (sensitive round-trip, etc.)
      # rather than each feature building its own infrastructure.
      checks.itest-nixops4-with-tfproto =
        pkgs.callPackage ../test/integration-test-nixops4-with-tfproto/check.nix
          {
            inherit inputs;
            nixops4 = inputs'.nixops4.packages.nixops4;
            flake-in-a-bottle = inputs'.nixops4.packages.flake-in-a-bottle;
            die = inputs'.nixops4.packages.die;
            adapter = config.packages.default;
            fixture = config.packages.fixture;
            # Only the lib module is needed at eval time inside the
            # sandbox flake — see the flake stub's header for why we
            # avoid exposing tfproto as a flake input.
            nixops4-tfproto-lib = ../nix/lib/lib.nix;
            # Materialise the schema once, outside the sandbox where
            # network is available.
            fixture-schema = self.lib.materialiseProviderSchema {
              inherit (pkgs) runCommand;
              adapterExecutable = "${config.packages.default}/bin/nixops4-resources-tfproto";
              providerExecutable = "${config.packages.fixture}/bin/fixture";
            };
          };

      # Guard against silent drift between the snapshot the nix-unit
      # tests read and the fixture provider's live schema.
      checks.fixture-schema-snapshot =
        let
          live = self.lib.materialiseProviderSchema {
            inherit (pkgs) runCommand;
            adapterExecutable = "${config.packages.default}/bin/nixops4-resources-tfproto";
            providerExecutable = "${config.packages.fixture}/bin/fixture";
          };
        in
        pkgs.runCommand "fixture-schema-snapshot-check" { } ''
          if diff -u ${../nix/tests/fixture-schema.json} ${live} > $out; then
            :
          else
            echo
            echo "fixture provider schema differs from the checked-in snapshot."
            echo "regenerate with:"
            echo "  nix eval --raw .#lib.materialiseProviderSchema \\"
            echo "    --apply 'f: f { runCommand = ...; adapterExecutable = ...; providerExecutable = ...; }' \\"
            echo "    | xargs cat > nix/tests/fixture-schema.json"
            exit 1
          fi
        '';
      nix-unit.inputs = {
        inherit (inputs)
          flake-parts
          nix-cargo-integration
          nixops4
          nixpkgs
          ;
        "flake-parts/nixpkgs-lib" = inputs.flake-parts.inputs.nixpkgs-lib;
      };
      nix-unit.allowNetwork = true;

      pre-commit.settings.hooks.nixfmt.enable = true;
      pre-commit.settings.hooks.rustfmt.enable = true;
      pre-commit.settings.hooks.gofmt.enable = true;
      pre-commit.settings.settings.rust.cargoManifestPath = "./rust/Cargo.toml";

      # Enforce ///-style doc comments in Rust: rustfmt on stable won't.
      pre-commit.settings.hooks.rust-doc-comments = {
        enable = true;
        files = "\\.rs$";
        entry = "${pkgs.writeScript "rust-doc-comments" ''
          #!${pkgs.runtimeShell}
          set -uxo pipefail
          grep -n -C3 --color=always -F '/**' "$@"
          r=$?
          set -e
          if [ $r -eq 0 ]; then
            echo "Please replace /**-style comments by /// style comments in Rust code."
            exit 1
          fi
        ''}";
      };

      devShells.default = pkgs.mkShell {
        name = "nixops4-tfproto-devshell";
        strictDeps = true;
        inputsFrom = [ config.nci.outputs.nixops4-tfproto.devShell ];
        nativeBuildInputs = [
          inputs'.nix-unit.packages.default
          pkgs.gdb
          pkgs.go
          pkgs.gopls
          pkgs.hci
          pkgs.nixfmt
          pkgs.pkg-config
          pkgs.protobuf
          pkgs.rust-analyzer
          pkgs.rustfmt
        ];
        shellHook = config.pre-commit.installationScript;
        # rust-analyzer resolves nixpkgs via NIX_PATH.
        NIX_PATH = "nixpkgs=${inputs.nixpkgs}";
      };
    };
  hercules-ci.flake-update = {
    enable = true;
    baseMerge.enable = true;
    autoMergeMethod = "merge";
    # Run a day after nixops4's flake-update so freshly locked inputs
    # there are already available upstream when this repo bumps.
    when = {
      dayOfMonth = 3;
    };
    flakes = {
      "." = { };
      "dev" = { };
    };
  };
  herculesCI = _hci: {
    ciSystems = [
      "x86_64-linux"
      "aarch64-darwin"
    ];
  };

}
