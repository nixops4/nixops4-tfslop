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
      nix-unit.tests.lib = import ../nix/tests/lib.nix {
        inherit (pkgs) lib;
        inherit self;
        nixops4 = inputs.nixops4;
      };
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
