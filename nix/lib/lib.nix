# Public functions exposed as `nixops4-tfproto.lib.<name>`.
{ lib }:
let
  inherit (lib) mkOption types;

  # Map a go-cty JSON-encoded type to a NixOS module option type.
  #
  # The `type` field is produced by go-cty's `cty.Type` marshaller:
  # primitives are JSON strings (`"string"`, quotes included),
  # composites are JSON arrays (`["list","string"]`,
  # `["object",{…}]`). See <https://github.com/zclconf/go-cty> and
  # <https://github.com/zclconf/go-cty/blob/main/cty/json/marshal.go>
  # (`MarshalType` / `UnmarshalType`).
  #
  # Parse the JSON once, then dispatch structurally on the parsed
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
    # cty `dynamic` (aka "any"): the provider accepts any shape and
    # decides validity at Plan. Nix has no meaningful check to apply
    # upstream; `types.raw` passes the value through untouched.
    else if parsed == "dynamic" then
      types.raw
    else if builtins.isList parsed && builtins.length parsed == 2 && builtins.head parsed == "list" then
      types.listOf (ctyParsedToOptionType (builtins.elemAt parsed 1))
    # cty `map(T)`: dynamic string keys, uniform value type. Maps to
    # `attrsOf` — the same "arbitrary keys" nix concept.
    else if builtins.isList parsed && builtins.length parsed == 2 && builtins.head parsed == "map" then
      types.attrsOf (ctyParsedToOptionType (builtins.elemAt parsed 1))
    # cty `set(T)`: unordered, unique-valued collection. nixpkgs has
    # no distinct set type, so translate to listOf T and deduplicate
    # at apply. Terraform's set semantics don't care about order or
    # duplicates, and canonicalising avoids noisy state diffs when
    # equivalent configs happen to serialise differently.
    else if builtins.isList parsed && builtins.length parsed == 2 && builtins.head parsed == "set" then
      types.listOf (ctyParsedToOptionType (builtins.elemAt parsed 1))
    # cty `tuple`: ordered, fixed-length, heterogeneous element
    # types. nixpkgs has no natural heterogeneous-list type, so
    # translate to `types.listOf types.raw`; per-position type
    # enforcement happens on the provider side at Plan. Length is
    # asserted at apply time.
    else if
      builtins.isList parsed && builtins.length parsed == 2 && builtins.head parsed == "tuple"
    then
      types.listOf types.raw
    # cty `object` type: the attribute's *value* is a fixed-shape
    # struct. Serialised as `["object", {field: type, …}]`.
    # Structurally the same target as a Single-nesting block minus
    # the `nullOr` — one attribute entry, one atomic value, but
    # named sub-fields at declared types.
    else if
      builtins.isList parsed && builtins.length parsed == 2 && builtins.head parsed == "object"
    then
      let
        fields = builtins.elemAt parsed 1;
      in
      types.submodule {
        options = lib.mapAttrs (
          _name: fieldType:
          mkOption {
            type = ctyParsedToOptionType fieldType;
          }
        ) fields;
      }
    else
      throw "tfCommonSchemaToModule: unsupported cty type ${builtins.toJSON parsed}";

  # A parsed cty type is a set iff its serialisation starts with "set".
  # Used to attach `apply = lib.unique` on options whose value is a
  # set — nixpkgs has no set type, so the canonicalisation happens at
  # the option level.
  ctyIsSet =
    parsed: builtins.isList parsed && builtins.length parsed == 2 && builtins.head parsed == "set";

  # Fixed length of a cty tuple, or null if the parsed type isn't a
  # tuple. Used to attach a length-assertion `apply` on options whose
  # value is a tuple.
  ctyTupleLength =
    parsed:
    if builtins.isList parsed && builtins.length parsed == 2 && builtins.head parsed == "tuple" then
      builtins.length (builtins.elemAt parsed 1)
    else
      null;

  # Compose the option-level `apply` (dedup / length-check / passthrough)
  # implied by the parsed cty type. Only one applies for a given
  # attribute, so a simple if/else on the type kind is enough.
  ctyApplyForType =
    name: parsed:
    if ctyIsSet parsed then
      { apply = lib.unique; }
    else if ctyTupleLength parsed != null then
      let
        expected = ctyTupleLength parsed;
      in
      {
        apply =
          v:
          let
            got = builtins.length v;
          in
          if got != expected then
            throw "attribute ${name}: tuple length ${toString expected} required, got ${toString got}"
          else
            v;
      }
    else
      { };

  convertInputAttributes =
    attrs:
    lib.mapAttrs (
      name: attr:
      let
        parsed = builtins.fromJSON attr.type;
      in
      mkOption (
        {
          type = ctyParsedToOptionType parsed;
          description = if attr.description or "" != "" then attr.description else "Terraform input";
        }
        // ctyApplyForType name parsed
      )
    ) (lib.filterAttrs (_: attr: attr.required or false || attr.optional or false) attrs);

  convertOutputAttributes =
    attrs:
    lib.mapAttrs (
      name: attr:
      let
        parsed = builtins.fromJSON attr.type;
      in
      mkOption (
        {
          type = ctyParsedToOptionType parsed;
          description = if attr.description or "" != "" then attr.description else "Terraform output";
          readOnly = true;
        }
        // ctyApplyForType name parsed
      )
    ) (lib.filterAttrs (_: attr: attr.computed or false) attrs);

  # Nested blocks recurse into their own inner block; the inner block
  # may itself have attributes and further nested blocks.
  #
  # `min_items = 0` (the default, omitted from the schema) marks the block
  # as optional: Single becomes `nullOr submodule` with `default null`,
  # List becomes `listOf submodule` with `default [ ]`. `min_items >= 1`
  # drops the default so the user must supply a value.
  #
  # For List blocks, `min_items` and `max_items` are enforced via an
  # `apply` check that throws at eval time, so a bad length surfaces
  # before we spawn the provider. On Single nesting `max_items` is
  # noise (the block already caps at one) and is not enforced.
  convertInputBlockTypes =
    blockTypes:
    lib.mapAttrs (
      name: bt:
      let
        inner = types.submodule {
          options = convertBlockInputs bt.block;
        };
        minItems = bt.min_items or 0;
        maxItems = bt.max_items or 0;
        required = minItems >= 1;
        enforceLength =
          v:
          let
            len = builtins.length v;
          in
          if minItems > 0 && len < minItems then
            throw "block ${name}: at least ${toString minItems} entries required, got ${toString len}"
          else if maxItems > 0 && len > maxItems then
            throw "block ${name}: at most ${toString maxItems} entries allowed, got ${toString len}"
          else
            v;
      in
      mkOption (
        {
          type =
            if bt.nesting == "Single" then
              (if required then inner else types.nullOr inner)
            else if bt.nesting == "List" || bt.nesting == "Set" then
              types.listOf inner
            else if bt.nesting == "Map" then
              types.attrsOf inner
            else
              throw "tfCommonSchemaToModule: unsupported block nesting ${builtins.toJSON bt.nesting}";
          description = "Nested block ${name}";
        }
        // lib.optionalAttrs (!required) {
          default =
            if bt.nesting == "Single" then
              null
            else if bt.nesting == "Map" then
              { }
            else
              [ ];
        }
        // lib.optionalAttrs (bt.nesting == "List" && (minItems > 0 || maxItems > 0)) {
          apply = enforceLength;
        }
        // lib.optionalAttrs (bt.nesting == "Set") {
          apply = lib.unique;
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
          else if bt.nesting == "List" || bt.nesting == "Set" then
            types.listOf inner
          else if bt.nesting == "Map" then
            types.attrsOf inner
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
