# nixops4-tfproto

**Status:** alpha, active development. Interfaces will change.

A [NixOps4](https://nixops.dev/) resource-provider adapter for the tfplugin
gRPC protocol. Lets NixOps4 drive resources through existing providers built
for [Terraform](https://www.terraform.io/) and
[OpenTofu](https://opentofu.org/).

The name is a coinage — read it as "tfplugin protocol adapter", not "Terraform
CLI integration". Both Terraform and OpenTofu providers speak the same gRPC
protocol; this adapter targets that protocol directly.

## What this repo contains

- `rust/nixops4-resources-tfproto/` — the adapter binary (Rust). Speaks the
  tfplugin gRPC protocol on one side and the NixOps4 resource protocol on
  the other.
- `fixture/` — an intentional test-only tfplugin provider (Go), built to
  cover the protocol surface. Not a real-world provider; not distributed.
- `nix/` — Nix expressions: `tfProviderToModule` (translates a provider's
  schema into a NixOps4 provider module), tests, and build glue.

## What this repo does not contain

- A user-facing CLI. Adapter is spawned by NixOps4 as a resource provider.
- Support for the Terraform / OpenTofu CLI or state file formats. Only the
  plugin protocol.
- Vendored real-world providers. Users bring their own; the adapter finds
  them via `nixpkgs.terraform-providers.*` or equivalent.

## Development

```shellSession
direnv allow
cargo build
nix flake check
```

## License

- Rust and Nix code: LGPL-2.1 (see [LICENSE](LICENSE)).
- Vendored `tfplugin*.proto` files: MPL-2.0, from
  [hashicorp/terraform](https://github.com/hashicorp/terraform) — see
  [NOTICE](NOTICE) for provenance.
