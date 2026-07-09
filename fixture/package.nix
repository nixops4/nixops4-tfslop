{ buildGoModule }:

buildGoModule (finalAttrs: {
  pname = "nixops4-tfproto-fixture";
  version = "0.0.1";

  src = ./.;

  vendorHash = "sha256-C46TYRTtZo1Y86vDIvE9Ivn4o+ooHgTkZNrWOUuccyk=";

  meta.mainProgram = "fixture";
  meta.description = ''
    Intentional test-fixture provider for the tfplugin gRPC protocol.
    Not a real Terraform / OpenTofu provider; used only to exercise the
    nixops4-tfproto adapter.
  '';
})
