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
}
