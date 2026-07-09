{
  description = "NixOps4 adapter for the tfplugin gRPC protocol (Terraform, OpenTofu)";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix-cargo-integration.url = "github:yusdacra/nix-cargo-integration";
    nix-cargo-integration.inputs.nixpkgs.follows = "nixpkgs";
    # Pinned to a `groups` branch on a personal fork that stacks unmerged
    # prerequisites (chore/error-messages, fix/shutdown-hang,
    # feat/isOptionalInputName, feat/nixops4-destroy, plus a golden-diff
    # workaround for render-provider-docs). Repoint to the upstream
    # commit once these land.
    nixops4.url = "git+https://scratch.notatio.nl/botbot/nixops4?ref=groups&rev=ae0ab4e6fc54bcf5f55bcd3eae3aa356a8b63a10";
    nixpkgs.follows = "nixops4/nixpkgs";
  };

  outputs =
    inputs@{ self, flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } (
      { lib, ... }:
      {
        imports = [
          inputs.nix-cargo-integration.flakeModule
          inputs.flake-parts.flakeModules.partitions
          inputs.flake-parts.flakeModules.touchup
          ./rust/nci.nix
        ];
        systems = [
          "x86_64-linux"
          "aarch64-linux"
          "x86_64-darwin"
          "aarch64-darwin"
        ];
        perSystem =
          { config, pkgs, ... }:
          {
            packages.default = config.packages.nixops4-resources-tfproto-release;
            packages.fixture = pkgs.callPackage ./fixture/package.nix { };
          };

        flake.lib = import ./nix/lib/lib.nix { inherit lib; };

        # Dev-only tooling is isolated behind a flake-parts partition
        # (see dev/flake-module.nix), so as not to bother consumers.
        partitionedAttrs.devShells = "dev";
        partitionedAttrs.checks = "dev";
        partitionedAttrs.tests = "dev"; # nix-unit
        partitions.dev.extraInputsFlake = ./dev;
        partitions.dev.module = {
          imports = [ ./dev/flake-module.nix ];
        };

        # nix-unit's auto-generated `checks.nix-unit` derivation has
        # no `nix` binary on PATH, which our IFD-using tests need to
        # build the fixture schema. Its flake module doesn't expose
        # nativeBuildInputs, so touch up the derivation after the fact.
        # The touchup lives here (not in the dev partition) because
        # partitions promotes `flake.checks` untouched — touchup only
        # applies at the flake that produces `processedFlake`.
        touchup.attr.checks.any =
          { attrName, ... }:
          {
            attr.nix-unit.finish =
              drv:
              drv.overrideAttrs (old: {
                nativeBuildInputs = old.nativeBuildInputs ++ [
                  inputs.nixpkgs.legacyPackages.${attrName}.nix
                ];
              });
          };
      }
    );
}
