# Public functions exposed as `nixops4-tfproto.lib.<name>`.
{ lib }:
let
  inherit (lib) mkOption types;

  # Map a go-cty JSON-encoded type to a NixOS module option type.
  #
  # The `type` field on the wire is the JSON encoding produced by
  # go-cty's `cty.Type` marshaller: primitives are JSON strings
  # (`"string"`, quotes included), composites are JSON arrays
  # (`["list","string"]`, `["object",{…}]`). See
  # <https://github.com/zclconf/go-cty> and
  # <https://github.com/zclconf/go-cty/blob/main/cty/json/marshal.go>
  # (`MarshalType` / `UnmarshalType`).
  #
  # A hard error, rather than a fallback to `types.raw`, forces us to
  # notice a new attribute shape the first time it appears instead of
  # silently mishandling every resource that uses it.
  ctyTypeToOptionType =
    ctyType:
    if ctyType == "\"string\"" then
      types.str
    else if ctyType == "\"bool\"" then
      types.bool
    # cty `number` is arbitrary-precision decimal. `types.int` is the
    # pragmatic mapping — most Terraform attributes marked `number` are
    # integers in practice (counts, ports, timeouts). If we ever hit a
    # provider that legitimately produces fractional numbers, split
    # into `types.int` vs `types.float` based on the schema hint or
    # widen to a more general type.
    else if ctyType == "\"number\"" then
      types.int
    else
      throw "tfCommonSchemaToModule: unsupported cty type ${builtins.toJSON ctyType}";

  convertInputAttributes =
    attrs:
    lib.mapAttrs (
      _name: attr:
      mkOption {
        type = ctyTypeToOptionType attr.type;
        description = if attr.description or "" != "" then attr.description else "Terraform input";
      }
    ) (lib.filterAttrs (_: attr: attr.required or false || attr.optional or false) attrs);

  convertOutputAttributes =
    attrs:
    lib.mapAttrs (
      _name: attr:
      mkOption {
        type = ctyTypeToOptionType attr.type;
        description = if attr.description or "" != "" then attr.description else "Terraform output";
        readOnly = true;
      }
    ) (lib.filterAttrs (_: attr: attr.computed or false) attrs);
in
rec {
  /**
    Translate a parsed tfplugin schema into a nixops4 provider module.

    # Arguments
    - `adapterExecutable`: path to the `nixops4-resources-tfproto` binary.
    - `providerExecutable`: path to the target tfplugin provider binary.
    - `schema`: attrset parsed from `nixops4-resources-tfproto schema`.

    # Returns
    A module suitable for `providers.<name>` in a nixops4 deployment.
  */
  tfCommonSchemaToModule =
    {
      adapterExecutable,
      providerExecutable,
      schema,
    }:
    { ... }:
    {
      resourceTypes = lib.mapAttrs (typeName: value: {
        provider.executable = adapterExecutable;
        provider.args = [
          "run"
          "--provider-exe"
          providerExecutable
        ];
        # Every tfplugin resource is externally stateful (Terraform
        # tracks it in the state file). nixops4 needs an explicit state
        # provider on our side to mirror that.
        requireState = true;
        description =
          if value.block.description or "" != "" then
            value.block.description
          else
            "Terraform resource ${typeName}";
        inputs = {
          options = convertInputAttributes value.block.attributes;
        };
        outputs = {
          options = convertOutputAttributes value.block.attributes;
        };
      }) schema.resource_schemas;
    };

  /**
    Build a derivation whose `$out` is the go-cty JSON schema of a
    tfplugin provider, as produced by `nixops4-resources-tfproto schema`.

    # Arguments
    - `runCommand`: from the target system's Nixpkgs.
    - `adapterExecutable`: path to the `nixops4-resources-tfproto` binary.
    - `providerExecutable`: path to the target tfplugin provider binary.
  */
  materialiseProviderSchema =
    {
      runCommand,
      adapterExecutable,
      providerExecutable,
    }:
    runCommand "nixops4-tfproto-provider-schema" { }
      "${adapterExecutable} schema --provider-exe ${providerExecutable} > $out";

  /**
    Translate a tfplugin provider binary into a nixops4 provider module
    end-to-end (materialise the schema, then translate). Same interface
    as `materialiseProviderSchema` plus what `tfCommonSchemaToModule`
    needs beyond the schema itself.
  */
  tfProviderToModule =
    {
      runCommand,
      adapterExecutable,
      providerExecutable,
    }:
    let
      schemaDrv = materialiseProviderSchema {
        inherit runCommand adapterExecutable providerExecutable;
      };
      schema = builtins.fromJSON (builtins.readFile "${schemaDrv}");
    in
    tfCommonSchemaToModule {
      inherit adapterExecutable providerExecutable schema;
    };
}
