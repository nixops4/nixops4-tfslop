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
    specialArgs.resourceProviderSystem =
      throw "translation should not depend on resourceProviderSystem";
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
}
