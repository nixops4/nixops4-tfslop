# `modules.nixops4Provider.terraform`: a nixops4 provider module that
# turns a Terraform provider package into `providers.<name>`.
#
# Import it into a provider and name the package; everything else has a
# default: the binary's path inside a nixpkgs-style package, the adapter
# for the system nixops4 runs on, and the schema, materialised from the
# binary when no snapshot is given. All defaults can be overridden, so a
# provider that is not from nixpkgs, or a snapshot checked in to avoid
# the import from derivation, is a one-line change.
#
#   providers.scaleway = {
#     imports = [ inputs.nixops4-tfproto.modules.nixops4Provider.terraform ];
#     terraform.package = pkgs.terraform-providers.scaleway_scaleway;
#   };
thisFlake@{ withSystem, tfLib }:
{
  config,
  lib,
  resourceProviderSystem,
  ...
}:
let
  inherit (lib) mkOption types;
  cfg = config.terraform;
  forSystem = f: thisFlake.withSystem resourceProviderSystem f;
in
{
  options.terraform = {
    package = mkOption {
      type = types.nullOr types.package;
      default = null;
      description = ''
        A Terraform provider package as nixpkgs' `terraform-providers.*`
        build them: the binary lives under
        `libexec/terraform-providers/<registry address>/<version>/<os>_<arch>/`
        and the package carries `provider-source-address`, `repo` and
        `version`. Used only for the default of `executable`.
      '';
    };
    executable = mkOption {
      type = types.str;
      default =
        let
          p = cfg.package;
          osArch = forSystem ({ pkgs, ... }: "${pkgs.go.GOOS}_${pkgs.go.GOARCH}");
        in
        if p == null then
          throw "providers.<name>.terraform: set `package` or `executable`"
        else
          "${p}/libexec/terraform-providers/${p.provider-source-address}/${p.version}/${osArch}/${p.repo}_${p.version}";
      defaultText = lib.literalMD "the binary inside `package`";
      description = "Path to the tfplugin provider binary.";
    };
    adapter = mkOption {
      type = types.str;
      default = forSystem ({ config, ... }: "${config.packages.default}/bin/nixops4-resources-tfproto");
      defaultText = lib.literalMD "nixops4-tfproto's adapter for the system nixops4 runs on";
      description = "Path to the `nixops4-resources-tfproto` binary.";
    };
    schema = mkOption {
      type = types.attrs;
      default = builtins.fromJSON (
        builtins.readFile (
          forSystem (
            { pkgs, ... }:
            tfLib.materialiseProviderSchema {
              inherit (pkgs) runCommand;
              adapterExecutable = cfg.adapter;
              providerExecutable = cfg.executable;
            }
          )
        )
      );
      defaultText = lib.literalMD "materialised from the binary (an import from derivation)";
      description = ''
        The provider schema as `nixops4-resources-tfproto schema` prints
        it. Pass a checked-in snapshot to keep evaluation free of builds.
      '';
    };
  };

  config = {
    inherit
      (tfLib.tfCommonSchemaToModule {
        adapterExecutable = cfg.adapter;
        providerExecutable = cfg.executable;
        inherit (cfg) schema;
      } { })
      resourceTypes
      ;
  };
}
