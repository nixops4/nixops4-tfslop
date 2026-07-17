{
  description = "NixOps4 adapter for the tfplugin gRPC protocol (Terraform, OpenTofu)";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix-cargo-integration.url = "github:yusdacra/nix-cargo-integration";
    nix-cargo-integration.inputs.nixpkgs.follows = "nixpkgs";
    nixops4.url = "git+https://scratch.notatio.nl/botbot/nixops4?ref=for-tfproto&rev=e8c3ddfeff585db9823c02899fe2ba97cc543f10";
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
