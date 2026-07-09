{
  perSystem =
    { config, pkgs, ... }:
    {
      # https://flake.parts/options/nix-cargo-integration
      nci.projects.nixops4-tfproto = {
        path = ./.;
        profiles = {
          dev.drvConfig.env.RUSTFLAGS = "-D warnings";
          release.runTests = true;
        };
      };

      # `nixops4-resources-tfproto` needs `protoc` to generate the tfplugin
      # gRPC bindings at build time. The integration tests spawn the Go
      # fixture built by the same flake, so point them at it via env var —
      # `tests/handshake.rs` self-skips when the variable is unset.
      nci.crates.nixops4-resources-tfproto.drvConfig = {
        env.NIXOPS4_TFPROTO_FIXTURE = "${config.packages.fixture}/bin/fixture";
        mkDerivation = {
          nativeBuildInputs = [ pkgs.protobuf ];
          meta.mainProgram = "nixops4-resources-tfproto";
        };
      };
    };
}
