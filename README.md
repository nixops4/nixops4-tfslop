# nixops4-tfslop / nixops4-tfproto prototype

**Status:** alpha, active development. Interfaces will change.

This repo is vibe-coded, and has seen limited testing.

This will be thoroughly reviewed and deslopped, with confirmed progress merged
in a separate repo (`nixops4/nixops4-tfproto`, TBD).

You probably need an accompanying [nixops4 tfslop branch](https://github.com/nixops4/nixops4/branches/all?query=tfslop&lastTab=overview) that may or may not be complete and/or functional.

A [NixOps4](https://nixops.dev/) resource-provider adapter for the tfplugin
gRPC protocol. Lets NixOps4 drive resources through existing providers built
for [Terraform](https://www.terraform.io/) and
[OpenTofu](https://opentofu.org/).

The name is a coinage — read it as "tfplugin protocol adapter", not "Terraform
CLI integration". Both Terraform and OpenTofu providers speak the same gRPC
protocol; this adapter targets that protocol directly.

## Refresh is separate from apply

Terraform's canonical apply cycle is **Refresh → Plan → Apply**: every apply
starts by calling `ReadResource` on each resource in state to reconcile
against reality, then computes a diff, then applies it. Refresh is the slow
part.

NixOps4 is IaC-first: state is authoritative, and `nixops4 apply` does not
refresh. This adapter follows that discipline — it drives providers through
`ConfigureProvider → PlanResourceChange → ApplyResourceChange` on create and
update, and skips `ReadResource` entirely. Applies stay fast.

Drift reconciliation belongs in a separate operation, ideally event-driven:
a restart or reboot triggers a refresh (via a `nixops4 run`-mediated
runtime, a monitoring loop, or a boot-time hook), not every deployment.

Terraform users can approximate the same split with its own flags:
`terraform apply -refresh=false` matches what this adapter does by default,
and `terraform plan -refresh-only` matches the separate reconciliation
operation. The difference is that here it's the default posture rather than
opt-out — deployment is a distinct workflow from drift monitoring.

## Deferred plans don't apply

The tfplugin protocol lets providers return `Deferred` on
`PlanResourceChange` — "not enough info to decide yet." Terraform's
multi-cycle planner uses that: resources with unknown inputs are held out
of the current apply set and re-planned once their dependencies' outputs
land.

NixOps4 has no such planner. Nix evaluation completes before the adapter
runs, so the config passed to Plan is already the complete set — there
are no unknowns that could be "resolved later." The adapter announces
`ClientCapabilities.deferral_allowed = false` and rejects any Deferred
response as a protocol violation, surfacing the reason to the user.

The proto reflects the same invariant on the other side: only Plan can
be Deferred; `ApplyResourceChange.Response` has no `deferred` field at
all, because Apply-on-Deferred-Plan is out of contract by design. Apply
is either fully committed or errored — never partial.

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

## Provider configuration

Every generated resource type takes `inputs.provider`, typed from the
provider's own configuration block (credentials, region, zone, ...).
The adapter takes it out of the inputs before Plan and hands it to
`ConfigureProvider`; a resource is destroyed with the configuration it
was created with. The name is safe because Terraform reserves
`provider` as a meta-argument on every resource, so no schema can have
an attribute called that.

```nix
members.key = {
  type = providers.scaleway.scaleway_iam_ssh_key;
  inputs.provider = { zone = "nl-ams-1"; secret_key = mkSensitive { … } "…"; };
  inputs.name = "deployer";
  inputs.public_key = "ssh-ed25519 …";
};
```

Attributes left unset fall back to whatever the provider reads on its
own, typically environment variables. There is deliberately no
provider-wide default: configuration is data on the resource, and
sharing it between resources is the expression's job.

## Using a provider from nixpkgs

`modules.nixops4Provider.terraform` turns a Terraform provider package
into a nixops4 provider. Name the package; the binary path, the adapter
for the system nixops4 runs on, and the schema (materialised from the
binary) all have defaults, each overridable:

```nix
providers.scaleway = {
  imports = [ inputs.nixops4-tfproto.modules.nixops4Provider.terraform ];
  terraform.package = pkgs.terraform-providers.scaleway_scaleway;
  # terraform.schema = lib.importJSON ./scaleway-schema.json;  # skip the IFD
};
```

`lib.tfProviderToModule` and `lib.tfCommonSchemaToModule` remain for
callers that want the function form.
