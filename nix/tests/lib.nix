# nix-unit tests for `nixops4-tfproto.lib`.
{
  lib,
  runCommand,
  self,
  nixops4,
  adapter,
  fixture,
}:
let
  adapterExecutable = "${adapter}/bin/nixops4-resources-tfproto";
  providerExecutable = "${fixture}/bin/fixture";

  schemaDrv = self.lib.materialiseProviderSchema {
    inherit runCommand adapterExecutable providerExecutable;
  };
  fixtureSchema = builtins.fromJSON (builtins.readFile "${schemaDrv}");

  translated = self.lib.tfCommonSchemaToModule {
    adapterExecutable = "/fake/adapter";
    providerExecutable = "/fake/provider";
    schema = fixtureSchema;
  };

  # Evaluate the translated module as a nixops4Provider, mirroring how
  # nixops4 would consume it. We reuse nixops4's own provider.nix so the
  # test breaks loudly if that interface drifts.
  providerEval = lib.evalModules {
    modules = [
      "${nixops4.outPath}/nix/provider/provider.nix"
      { name = "fixture"; }
      { options._module.args = lib.mkOption { internal = true; }; }
      translated
    ];
    specialArgs.resourceProviderSystem = throw "translation should not depend on resourceProviderSystem";
  };

  cfg = providerEval.config;

  # Extract the `inputs` options declared for each fixture resource so
  # we can inspect the option types the translation emitted.
  fxStringInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_string.inputs ];
    }).options;
  fxBoolInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_bool.inputs ];
    }).options;
  fxNumberInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_number.inputs ];
    }).options;
  fxListStringInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_list_string.inputs ];
    }).options;
  fxObjectInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_object.inputs ];
    }).options;
  fxMapStringInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_map_string.inputs ];
    }).options;
  fxSetStringInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_set_string.inputs ];
    }).options;
  # Feed a list with duplicates: the apply-time dedup must canonicalise
  # it so the on-wire value doesn't carry noise into state diffs.
  materialisedSet =
    (lib.evalModules {
      modules = [
        cfg.resourceTypes.fx_set_string.inputs
        {
          value = [
            "a"
            "b"
            "a"
          ];
        }
      ];
    }).config;
  materialisedMap =
    (lib.evalModules {
      modules = [
        cfg.resourceTypes.fx_map_string.inputs
        {
          value = {
            env = "prod";
            team = "platform";
          };
        }
      ];
    }).config;
  # Materialisation: a valid object value must round-trip through the
  # option system without triggering type errors.
  materialisedObject =
    (lib.evalModules {
      modules = [
        cfg.resourceTypes.fx_object.inputs
        {
          settings = {
            endpoint = "https://example.invalid";
            port = 443;
          };
        }
      ];
    }).config;
  fxSingleBlockInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_single_block.inputs ];
    }).options;
  fxListBlockInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_list_block.inputs ];
    }).options;
  fxOptionalBlockAttrInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_optional_block_attr.inputs ];
    }).options;
  fxNestedBlocksInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_nested_blocks.inputs ];
    }).options;
  fxRequiredSingleBlockInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_required_single_block.inputs ];
    }).options;
  fxComputedBlockAttrInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_computed_block_attr.inputs ];
    }).options;
  fxComputedBlockAttrOutputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_computed_block_attr.outputs ];
    }).options;
  fxComputedOutputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_computed.outputs ];
    }).options;
  fxComputedInputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_computed.inputs ];
    }).options;
  # Materialise a value through the output side: nixops4 writes into
  # `outputs.<name>` on Apply. This proves the option accepts the
  # shape the projection actually produces.
  materialisedComputedOutputs =
    (lib.evalModules {
      modules = [
        cfg.resourceTypes.fx_computed.outputs
        { id = "computed-nixops4"; }
      ];
    }).config;

  # MinItems / MaxItems enforcement on List blocks — a bad length must
  # fail at eval, before any provider is spawned.
  tryEvalRuleCount =
    typeName: n:
    builtins.tryEval (
      (lib.evalModules {
        modules = [
          cfg.resourceTypes.${typeName}.inputs
          {
            rule = builtins.genList (i: {
              name = "r${toString i}";
            }) n;
          }
        ];
      }).config.rule
    );
  # Sibling: same shape without a computed inner attribute. Confirms
  # that a block-with-no-computed-content is *absent* from outputs.
  fxSingleBlockOutputOptions =
    (lib.evalModules {
      modules = [ cfg.resourceTypes.fx_single_block.outputs ];
    }).options;

  # Materialisation of a MinItems=1 Single block: the option must
  # accept a supplied value and expose no default.
  materialisedRequiredSingle =
    (lib.evalModules {
      modules = [
        cfg.resourceTypes.fx_required_single_block.inputs
        { settings.endpoint = "https://example.invalid"; }
      ];
    }).config;

  # Materialisation checks: build a real config through the module
  # system to prove the option types not only *exist* with the right
  # shape but actually *accept* a valid value.
  materialisedSingleBlock =
    (lib.evalModules {
      modules = [
        cfg.resourceTypes.fx_single_block.inputs
        { settings.endpoint = "https://example.invalid"; }
      ];
    }).config;
  materialisedListBlock =
    (lib.evalModules {
      modules = [
        cfg.resourceTypes.fx_list_block.inputs
        {
          rule = [
            { name = "first"; }
            { name = "second"; }
          ];
        }
      ];
    }).config;
  materialisedNested =
    (lib.evalModules {
      modules = [
        cfg.resourceTypes.fx_nested_blocks.inputs
        { outer.inner.value = "deep"; }
      ];
    }).config;

in
{
  "test resource type present" = {
    expr = cfg.resourceTypes ? fx_string;
    expected = true;
  };

  "test resource type provider args" = {
    expr = cfg.resourceTypes.fx_string.provider.args;
    expected = [
      "run"
      "--provider-exe"
      "/fake/provider"
    ];
  };

  "test resource type requires state" = {
    expr = cfg.resourceTypes.fx_string.requireState;
    expected = true;
  };

  "test resource type description" = {
    expr = cfg.resourceTypes.fx_string.description;
    expected = "Terraform resource fx_string";
  };

  "test string attribute becomes types.str" = {
    expr = fxStringInputOptions.value.type.name;
    expected = "str";
  };

  "test string attribute description comes from schema" = {
    expr = fxStringInputOptions.value.description;
    expected = "A required string. Echoed back on create.";
  };

  "test bool attribute becomes types.bool" = {
    expr = fxBoolInputOptions.value.type.name;
    expected = "bool";
  };

  "test bool attribute description comes from schema" = {
    expr = fxBoolInputOptions.value.description;
    expected = "A required bool. Echoed back on create.";
  };

  "test number attribute becomes types.int" = {
    expr = fxNumberInputOptions.value.type.name;
    expected = "int";
  };

  "test number attribute description comes from schema" = {
    expr = fxNumberInputOptions.value.description;
    expected = "A required number. Echoed back on create.";
  };

  "test list-of-string attribute becomes types.listOf types.str" = {
    expr = fxListStringInputOptions.value.type.name;
    expected = "listOf";
  };

  "test list-of-string element type is types.str" = {
    expr = fxListStringInputOptions.value.type.functor.payload.elemType.name;
    expected = "str";
  };

  "test list-of-string description comes from schema" = {
    expr = fxListStringInputOptions.value.description;
    expected = "A required list of strings. Echoed back on create.";
  };

  # cty `object` attribute → submodule with typed sub-options. The
  # whole value is atomic on the schema side (one attribute entry, one
  # set of flags), but its interior shape is fixed at schema time.

  "test object attribute becomes types.submodule" = {
    expr = fxObjectInputOptions.settings.type.name;
    expected = "submodule";
  };

  "test object attribute exposes typed sub-options" = {
    expr =
      let
        sub = fxObjectInputOptions.settings.type.getSubOptions [ ];
      in
      {
        endpoint = sub.endpoint.type.name;
        port = sub.port.type.name;
      };
    expected = {
      endpoint = "str";
      port = "int";
    };
  };

  "test object attribute value round-trips through option" = {
    expr = materialisedObject.settings;
    expected = {
      endpoint = "https://example.invalid";
      port = 443;
    };
  };

  # cty `map(T)` attribute → `types.attrsOf T`.

  "test map-of-string attribute becomes types.attrsOf" = {
    expr = fxMapStringInputOptions.value.type.name;
    expected = "attrsOf";
  };

  "test map-of-string element type is types.str" = {
    expr = fxMapStringInputOptions.value.type.functor.payload.elemType.name;
    expected = "str";
  };

  "test map-of-string value round-trips through option" = {
    expr = materialisedMap.value;
    expected = {
      env = "prod";
      team = "platform";
    };
  };

  # cty `set(T)` attribute → `types.listOf T` with dedup at apply.
  # Terraform's set semantics don't care about order or duplicates;
  # canonicalising here prevents state noise.

  "test set-of-string attribute becomes types.listOf" = {
    expr = fxSetStringInputOptions.value.type.name;
    expected = "listOf";
  };

  "test set-of-string element type is types.str" = {
    expr = fxSetStringInputOptions.value.type.functor.payload.elemType.name;
    expected = "str";
  };

  "test set-of-string dedupes duplicates" = {
    expr = materialisedSet.value;
    expected = [
      "a"
      "b"
    ];
  };

  # Nested blocks. A Single-nesting block becomes `nullOr submodule` so
  # the whole block can be omitted; a List-nesting block becomes
  # `listOf submodule`. In both cases the inner block's attributes
  # become options inside the submodule.

  "test single block becomes nullOr submodule" = {
    expr = fxSingleBlockInputOptions.settings.type.name;
    expected = "nullOr";
  };

  "test single block inner is a submodule" = {
    expr = fxSingleBlockInputOptions.settings.type.nestedTypes.elemType.name;
    expected = "submodule";
  };

  "test single block inner endpoint is str" = {
    expr =
      (fxSingleBlockInputOptions.settings.type.nestedTypes.elemType.getSubOptions [ ]).endpoint.type.name;
    expected = "str";
  };

  "test single block default is null" = {
    expr = fxSingleBlockInputOptions.settings.default;
    expected = null;
  };

  "test list block becomes listOf" = {
    expr = fxListBlockInputOptions.rule.type.name;
    expected = "listOf";
  };

  "test list block element is a submodule" = {
    expr = fxListBlockInputOptions.rule.type.functor.payload.elemType.name;
    expected = "submodule";
  };

  "test list block inner name is str" = {
    expr =
      (fxListBlockInputOptions.rule.type.functor.payload.elemType.getSubOptions [ ]).name.type.name;
    expected = "str";
  };

  "test list block default is empty list" = {
    expr = fxListBlockInputOptions.rule.default;
    expected = [ ];
  };

  # Optional attributes inside a block must survive the
  # required-or-optional filter.

  "test optional inner attribute is present in submodule" = {
    expr =
      (fxOptionalBlockAttrInputOptions.settings.type.nestedTypes.elemType.getSubOptions [ ]) ? endpoint;
    expected = true;
  };

  "test optional inner attribute is str" = {
    expr =
      (fxOptionalBlockAttrInputOptions.settings.type.nestedTypes.elemType.getSubOptions [ ])
      .endpoint.type.name;
    expected = "str";
  };

  # Recursive nesting: a block that contains another block that
  # contains an attribute. Two levels of `nullOr submodule` around the
  # eventual `types.str`.

  "test nested-nested outer is nullOr submodule" = {
    expr = fxNestedBlocksInputOptions.outer.type.name;
    expected = "nullOr";
  };

  "test nested-nested inner is nullOr submodule" = {
    expr =
      (fxNestedBlocksInputOptions.outer.type.nestedTypes.elemType.getSubOptions [ ]).inner.type.name;
    expected = "nullOr";
  };

  "test nested-nested value is str" = {
    expr =
      (
        (fxNestedBlocksInputOptions.outer.type.nestedTypes.elemType.getSubOptions [ ])
        .inner.type.nestedTypes.elemType.getSubOptions
        [ ]
      ).value.type.name;
    expected = "str";
  };

  # Materialisation: a valid config value passes through the option
  # system without triggering type errors, and the value round-trips.

  "test single block value round-trips through option" = {
    expr = materialisedSingleBlock.settings.endpoint;
    expected = "https://example.invalid";
  };

  "test list block value round-trips through option" = {
    expr = builtins.map (r: r.name) materialisedListBlock.rule;
    expected = [
      "first"
      "second"
    ];
  };

  "test recursive block value round-trips through option" = {
    expr = materialisedNested.outer.inner.value;
    expected = "deep";
  };

  # MinItems=1 on a Single block: the block itself is not nullable and
  # has no default, so the user must supply it. The submodule shape
  # inside is otherwise identical to the optional case.

  "test required single block is not nullOr" = {
    expr = fxRequiredSingleBlockInputOptions.settings.type.name;
    expected = "submodule";
  };

  "test required single block has no default" = {
    expr = fxRequiredSingleBlockInputOptions.settings ? default;
    expected = false;
  };

  "test required single block value round-trips through option" = {
    expr = materialisedRequiredSingle.settings.endpoint;
    expected = "https://example.invalid";
  };

  # Computed inner attributes surface as `outputs.<block>.<attr>`.
  # `endpoint` (required) is on the input side; `token` (computed) is
  # on the output side. The block appears in both.

  "test computed block attr endpoint is on input" = {
    expr =
      (fxComputedBlockAttrInputOptions.settings.type.nestedTypes.elemType.getSubOptions [ ]) ? endpoint;
    expected = true;
  };

  "test computed block attr token is not on input" = {
    expr =
      (fxComputedBlockAttrInputOptions.settings.type.nestedTypes.elemType.getSubOptions [ ]) ? token;
    expected = false;
  };

  "test computed block attr block is present on output" = {
    expr = fxComputedBlockAttrOutputOptions ? settings;
    expected = true;
  };

  "test computed block attr token is on output" = {
    expr =
      (fxComputedBlockAttrOutputOptions.settings.type.nestedTypes.elemType.getSubOptions [ ]) ? token;
    expected = true;
  };

  "test computed block attr token is readOnly" = {
    expr =
      (fxComputedBlockAttrOutputOptions.settings.type.nestedTypes.elemType.getSubOptions [ ])
      .token.readOnly;
    expected = true;
  };

  "test block with no computed content is absent from outputs" = {
    expr = fxSingleBlockOutputOptions ? settings;
    expected = false;
  };

  # A computed top-level attribute is exposed on the outputs side, not
  # inputs. The optional user input `value` stays on inputs only.

  "test computed top-level id is on outputs" = {
    expr = fxComputedOutputOptions ? id;
    expected = true;
  };

  "test computed top-level id is readOnly" = {
    expr = fxComputedOutputOptions.id.readOnly;
    expected = true;
  };

  "test computed top-level id is not on inputs" = {
    expr = fxComputedInputOptions ? id;
    expected = false;
  };

  "test computed top-level value is on inputs" = {
    expr = fxComputedInputOptions ? value;
    expected = true;
  };

  # Materialisation: nixops4 will assign into outputs on Apply. The
  # option must accept the shape the runtime projection produces.

  "test computed top-level output round-trips through option" = {
    expr = materialisedComputedOutputs.id;
    expected = "computed-nixops4";
  };

  # List MinItems enforcement.
  "test list block below MinItems fails" = {
    expr = (tryEvalRuleCount "fx_min_list_block" 1).success;
    expected = false;
  };
  "test list block at MinItems succeeds" = {
    expr = builtins.length (tryEvalRuleCount "fx_min_list_block" 2).value;
    expected = 2;
  };

  # List MaxItems enforcement.
  "test list block above MaxItems fails" = {
    expr = (tryEvalRuleCount "fx_max_list_block" 3).success;
    expected = false;
  };
  "test list block at MaxItems succeeds" = {
    expr = builtins.length (tryEvalRuleCount "fx_max_list_block" 2).value;
    expected = 2;
  };
}
