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
  # Parse the wire JSON once, then dispatch structurally on the parsed
  # value — otherwise the primitive arms compare against string
  # literals like `"\"string\""` and composite arms would have to
  # branch on prefix substrings, which is ugly.
  #
  # A hard error, rather than a fallback to `types.raw`, forces us to
  # notice a new attribute shape the first time it appears instead of
  # silently mishandling every resource that uses it.
  ctyTypeToOptionType = ctyType: ctyParsedToOptionType (builtins.fromJSON ctyType);

  ctyParsedToOptionType =
    parsed:
    if parsed == "string" then
      types.str
    else if parsed == "bool" then
      types.bool
    # cty `number` is arbitrary-precision decimal. `types.int` is the
    # pragmatic mapping — most Terraform attributes marked `number` are
    # integers in practice (counts, ports, timeouts). If we ever hit a
    # provider that legitimately produces fractional numbers, split
    # into `types.int` vs `types.float` based on the schema hint or
    # widen to a more general type.
    else if parsed == "number" then
      types.int
    else if builtins.isList parsed && builtins.length parsed == 2 && builtins.head parsed == "list" then
      types.listOf (ctyParsedToOptionType (builtins.elemAt parsed 1))
    else
      throw "tfCommonSchemaToModule: unsupported cty type ${builtins.toJSON parsed}";

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

  # Nested blocks recurse into their own inner block; the inner block
  # may itself have attributes and further nested blocks.
  #
  # `min_items = 0` (the default, omitted on the wire) marks the block
  # as optional: Single becomes `nullOr submodule` with `default null`,
  # List becomes `listOf submodule` with `default [ ]`. `min_items >= 1`
  # drops the default so the user must supply a value — module-system
  # validation surfaces a missing value as a "used but not defined"
  # error. We do not enforce list length; that becomes a runtime
  # diagnostic when tfplugin's Plan rejects the value.
  #
  # `max_items` is deliberately unused for now: tfplugin providers
  # commonly set it as a hint (e.g. `max_items = 1` on a Single-like
  # List), not a hard invariant. If we grow a use case we can add a
  # `check`/`apply` layer here.
  convertInputBlockTypes =
    blockTypes:
    lib.mapAttrs (
      name: bt:
      let
        inner = types.submodule {
          options = convertBlockInputs bt.block;
        };
        minItems = bt.min_items or 0;
        required = minItems >= 1;
      in
      mkOption (
        {
          type =
            if bt.nesting == "Single" then
              (if required then inner else types.nullOr inner)
            else if bt.nesting == "List" then
              types.listOf inner
            else
              throw "tfCommonSchemaToModule: unsupported block nesting ${builtins.toJSON bt.nesting}";
          description = "Nested block ${name}";
        }
        // lib.optionalAttrs (!required) {
          default = if bt.nesting == "Single" then null else [ ];
        }
      )
    ) blockTypes;

  convertBlockInputs =
    block:
    (convertInputAttributes (block.attributes or { }))
    // (convertInputBlockTypes (block.block_types or { }));

  # For outputs, mirror the block structure so provider-computed
  # attributes end up at their natural path (`outputs.<block>.<attr>`)
  # instead of collapsing to a flat map. A block appears in outputs
  # iff it contains a computed attribute at any depth: blocks that are
  # pure user-config have nothing to project and would only pollute
  # the output shape.
  blockHasComputed =
    block:
    lib.any (attr: attr.computed or false) (lib.attrValues (block.attributes or { }))
    || lib.any (bt: blockHasComputed bt.block) (lib.attrValues (block.block_types or { }));

  convertOutputBlockTypes =
    blockTypes:
    lib.mapAttrs (
      name: bt:
      let
        inner = types.submodule {
          options = convertBlockOutputs bt.block;
        };
        minItems = bt.min_items or 0;
        required = minItems >= 1;
      in
      mkOption {
        type =
          if bt.nesting == "Single" then
            (if required then inner else types.nullOr inner)
          else if bt.nesting == "List" then
            types.listOf inner
          else
            throw "tfCommonSchemaToModule: unsupported block nesting ${builtins.toJSON bt.nesting}";
        readOnly = true;
        description = "Nested block ${name}";
      }
    ) (lib.filterAttrs (_: bt: blockHasComputed bt.block) blockTypes);

  convertBlockOutputs =
    block:
    (convertOutputAttributes (block.attributes or { }))
    // (convertOutputBlockTypes (block.block_types or { }));
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
          options = convertBlockInputs value.block;
        };
        outputs = {
          options = convertBlockOutputs value.block;
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
