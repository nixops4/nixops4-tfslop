{
  description = "NixOps4 adapter for the tfplugin gRPC protocol (Terraform, OpenTofu)";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix-cargo-integration.url = "github:yusdacra/nix-cargo-integration";
    nix-cargo-integration.inputs.nixpkgs.follows = "nixpkgs";
    nixops4.url = "git+https://scratch.notatio.nl/botbot/nixops4?ref=for-tfproto&rev=bacc0cb2a83dc3a76df201a9a74cac8ca15dc858";
    nixpkgs.follows = "nixops4/nixpkgs";
  };

  outputs =
    inputs@{ self, flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } (
      {
        config,
        lib,
        withSystem,
        flake-parts-lib,
        ...
      }:
      {
        imports = [
          inputs.nix-cargo-integration.flakeModule
          inputs.flake-parts.flakeModules.partitions
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

        # A provider module that takes a Terraform provider package and
        # does the rest; see nix/provider-module.nix.
        flake.modules.nixops4Provider.terraform = flake-parts-lib.importApply ./nix/provider-module.nix {
          inherit withSystem;
          tfLib = config.flake.lib;
        };

        flake.lib = import ./nix/lib/lib.nix {
          inherit lib;
          # `nixops4Lib` gives the schema translator access to
          # `nixops4Lib.types.{acceptSensitive,sensitiveOf}`, needed
          # to generate option types for schema-declared-sensitive
          # attributes.
          nixops4Lib = inputs.nixops4.lib;
        };

        # Dev-only tooling is isolated behind a flake-parts partition
        # (see dev/flake-module.nix), so as not to bother consumers.
        partitionedAttrs.devShells = "dev";
        partitionedAttrs.checks = "dev";
        partitionedAttrs.tests = "dev"; # nix-unit
        partitions.dev.extraInputsFlake = ./dev;
        partitions.dev.module = {
          imports = [ ./dev/flake-module.nix ];
        };
      }
    );
}
