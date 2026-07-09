{ buildGoModule }:

buildGoModule (finalAttrs: {
  pname = "nixops4-tfproto-fixture";
  version = "0.0.1";

  src = ./.;

  # Placeholder module has no dependencies. When the fixture grows real
  # imports, replace `null` with the resulting `vendorHash`.
  vendorHash = null;

  meta.mainProgram = "fixture";
  meta.description = ''
    Intentional test-fixture provider for the tfplugin gRPC protocol.
    Not a real Terraform / OpenTofu provider; used only to exercise the
    nixops4-tfproto adapter.
  '';
})
