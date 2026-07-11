// Intentional test-fixture tfplugin provider.
//
// This program is not a real Terraform / OpenTofu provider. Its role is
// to exercise `nixops4-resources-tfproto` against the tfplugin gRPC
// protocol under controlled conditions: each protocol capability is
// implemented only insofar as tests require, so a failing test points
// at exactly one part of the surface. Unimplemented methods return an
// error diagnostic named after themselves.
package main

import (
	"context"
	"encoding/json"
	"os"

	"github.com/hashicorp/terraform-plugin-go/tfprotov6"
	"github.com/hashicorp/terraform-plugin-go/tfprotov6/tf6server"
	"github.com/hashicorp/terraform-plugin-go/tftypes"
)

func main() {
	err := tf6server.Serve("nixops4-tfproto-fixture", func() tfprotov6.ProviderServer {
		return &fixtureProvider{}
	})
	if err != nil {
		panic(err)
	}
}

type fixtureProvider struct{}

func notImplemented(op string) *tfprotov6.Diagnostic {
	return &tfprotov6.Diagnostic{
		Severity: tfprotov6.DiagnosticSeverityError,
		Summary:  "nixops4-tfproto fixture: " + op + " not implemented",
	}
}

// --- ProviderServer core ---

func (p *fixtureProvider) GetMetadata(context.Context, *tfprotov6.GetMetadataRequest) (*tfprotov6.GetMetadataResponse, error) {
	return &tfprotov6.GetMetadataResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("GetMetadata")},
	}, nil
}

func (p *fixtureProvider) GetProviderSchema(context.Context, *tfprotov6.GetProviderSchemaRequest) (*tfprotov6.GetProviderSchemaResponse, error) {
	return &tfprotov6.GetProviderSchemaResponse{
		ResourceSchemas: map[string]*tfprotov6.Schema{
			"fx_string": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "value",
							Type:            tftypes.String,
							Required:        true,
							Description:     "A required string. Echoed back on create.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			"fx_bool": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "value",
							Type:            tftypes.Bool,
							Required:        true,
							Description:     "A required bool. Echoed back on create.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			"fx_number": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "value",
							Type:            tftypes.Number,
							Required:        true,
							Description:     "A required number. Echoed back on create.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			"fx_list_string": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "value",
							Type:            tftypes.List{ElementType: tftypes.String},
							Required:        true,
							Description:     "A required list of strings. Echoed back on create.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			// cty `map(T)` attribute: dynamic string keys, uniform
			// value type. The `tags = { … }` pattern most providers
			// expose.
			"fx_map_string": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "value",
							Type:            tftypes.Map{ElementType: tftypes.String},
							Required:        true,
							Description:     "A required map of strings.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			// cty `object` attribute: the value is a fixed-shape
			// struct. Distinct from a nested block — the whole value
			// is atomic (one required/computed/write_only bit for the
			// whole thing), no nesting cardinality. Real providers
			// use this heavily for structured attributes.
			"fx_object": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name: "settings",
							Type: tftypes.Object{
								AttributeTypes: map[string]tftypes.Type{
									"endpoint": tftypes.String,
									"port":     tftypes.Number,
								},
							},
							Required:        true,
							Description:     "A required object attribute.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			// Single-nested block: at most one `settings { … }`.
			"fx_single_block": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					BlockTypes: []*tfprotov6.SchemaNestedBlock{
						{
							TypeName: "settings",
							Nesting:  tfprotov6.SchemaNestedBlockNestingModeSingle,
							Block: &tfprotov6.SchemaBlock{
								Attributes: []*tfprotov6.SchemaAttribute{
									{
										Name:            "endpoint",
										Type:            tftypes.String,
										Required:        true,
										Description:     "Where to talk to.",
										DescriptionKind: tfprotov6.StringKindPlain,
									},
								},
							},
						},
					},
				},
			},
			// Required Single-nested block: MinItems=1 means the user
			// must supply the block; it does not default to null.
			"fx_required_single_block": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					BlockTypes: []*tfprotov6.SchemaNestedBlock{
						{
							TypeName: "settings",
							Nesting:  tfprotov6.SchemaNestedBlockNestingModeSingle,
							MinItems: 1,
							Block: &tfprotov6.SchemaBlock{
								Attributes: []*tfprotov6.SchemaAttribute{
									{
										Name:            "endpoint",
										Type:            tftypes.String,
										Required:        true,
										Description:     "Where to talk to.",
										DescriptionKind: tfprotov6.StringKindPlain,
									},
								},
							},
						},
					},
				},
			},
			// Single-nested block with an optional inner attribute.
			// Exercises the `optional=true, required=false` filter branch
			// inside a nested block.
			"fx_optional_block_attr": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					BlockTypes: []*tfprotov6.SchemaNestedBlock{
						{
							TypeName: "settings",
							Nesting:  tfprotov6.SchemaNestedBlockNestingModeSingle,
							Block: &tfprotov6.SchemaBlock{
								Attributes: []*tfprotov6.SchemaAttribute{
									{
										Name:            "endpoint",
										Type:            tftypes.String,
										Optional:        true,
										Description:     "Where to talk to (optional).",
										DescriptionKind: tfprotov6.StringKindPlain,
									},
								},
							},
						},
					},
				},
			},
			// Single-nested block with a provider-computed inner
			// attribute. Exercises output-side projection through a
			// block: `outputs.settings.token` must exist while
			// `outputs.token` (flat) must not.
			"fx_computed_block_attr": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					BlockTypes: []*tfprotov6.SchemaNestedBlock{
						{
							TypeName: "settings",
							Nesting:  tfprotov6.SchemaNestedBlockNestingModeSingle,
							Block: &tfprotov6.SchemaBlock{
								Attributes: []*tfprotov6.SchemaAttribute{
									{
										Name:            "endpoint",
										Type:            tftypes.String,
										Required:        true,
										Description:     "Where to talk to.",
										DescriptionKind: tfprotov6.StringKindPlain,
									},
									{
										Name:            "token",
										Type:            tftypes.String,
										Computed:        true,
										Description:     "Provider-issued token.",
										DescriptionKind: tfprotov6.StringKindPlain,
									},
								},
							},
						},
					},
				},
			},
			// Block containing a block: exercises the recursion in the
			// Nix translation. `outer.inner.value` should end up as a
			// `types.str` two submodules deep.
			"fx_nested_blocks": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					BlockTypes: []*tfprotov6.SchemaNestedBlock{
						{
							TypeName: "outer",
							Nesting:  tfprotov6.SchemaNestedBlockNestingModeSingle,
							Block: &tfprotov6.SchemaBlock{
								BlockTypes: []*tfprotov6.SchemaNestedBlock{
									{
										TypeName: "inner",
										Nesting:  tfprotov6.SchemaNestedBlockNestingModeSingle,
										Block: &tfprotov6.SchemaBlock{
											Attributes: []*tfprotov6.SchemaAttribute{
												{
													Name:            "value",
													Type:            tftypes.String,
													Required:        true,
													Description:     "Innermost required string.",
													DescriptionKind: tfprotov6.StringKindPlain,
												},
											},
										},
									},
								},
							},
						},
					},
				},
			},
			// Provider-computed attribute: the user supplies `value`;
			// the fixture populates `id = "computed-" + value` at Plan
			// time. Exercises new_state projection onto declared
			// computed outputs.
			"fx_computed": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "value",
							Type:            tftypes.String,
							Optional:        true,
							Description:     "User input; echoed into id.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
						{
							Name:            "id",
							Type:            tftypes.String,
							Computed:        true,
							Description:     "Provider-issued identifier.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			// Diagnostic-free crash: the caller picks *which* RPC
			// exits (crashing_operation = "plan" | "apply") and *with
			// what status* (exit_code). Exercises the adapter's
			// ability to correlate the child process's exit with the
			// RPC error across both Plan and Apply. Two axes,
			// data-driven — encoding them into resource-type names
			// would produce a resource per combination.
			// Blocks forever in its Plan handler. Exercises the
			// adapter's per-RPC timeout.
			"fx_hang_on_plan": {Version: 0, Block: &tfprotov6.SchemaBlock{}},
			// Returns a Deferred response on Plan or Apply with a
			// caller-specified reason. Exercises the adapter's
			// surfacing of `Deferred` — providers can send it even
			// when the client announced `deferral_allowed = false`.
			"fx_defer": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "deferring_operation",
							Type:            tftypes.String,
							Required:        true,
							Description:     `"plan" or "apply".`,
							DescriptionKind: tfprotov6.StringKindPlain,
						},
						{
							Name:            "reason",
							Type:            tftypes.String,
							Required:        true,
							Description:     "One of RESOURCE_CONFIG_UNKNOWN, PROVIDER_CONFIG_UNKNOWN, ABSENT_PREREQ.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			// Plan handler errors unless the adapter announced
			// ClientCapabilities.WriteOnlyAttributesAllowed. Real
			// providers may silently drop or reject write-only
			// attributes when the client doesn't announce support.
			"fx_check_write_only_capability": {Version: 0, Block: &tfprotov6.SchemaBlock{}},
			"fx_die": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "crashing_operation",
							Type:            tftypes.String,
							Required:        true,
							Description:     `"plan" or "apply".`,
							DescriptionKind: tfprotov6.StringKindPlain,
						},
						{
							Name:            "exit_code",
							Type:            tftypes.Number,
							Required:        true,
							Description:     "Status the fixture calls os.Exit with.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			// Resource with a write_only attribute. `secret` must not
			// appear in prior_state — the adapter is supposed to strip
			// it. Fixture's Plan errors if it sees one.
			"fx_write_only": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					Attributes: []*tfprotov6.SchemaAttribute{
						{
							Name:            "value",
							Type:            tftypes.String,
							Optional:        true,
							Description:     "Persistent user input.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
						{
							Name:            "secret",
							Type:            tftypes.String,
							Optional:        true,
							WriteOnly:       true,
							Description:     "Write-only secret; never persisted.",
							DescriptionKind: tfprotov6.StringKindPlain,
						},
					},
				},
			},
			// List-nested block with MinItems=2: fewer entries must
			// fail before spawning the provider.
			"fx_min_list_block": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					BlockTypes: []*tfprotov6.SchemaNestedBlock{
						{
							TypeName: "rule",
							Nesting:  tfprotov6.SchemaNestedBlockNestingModeList,
							MinItems: 2,
							Block: &tfprotov6.SchemaBlock{
								Attributes: []*tfprotov6.SchemaAttribute{
									{
										Name:            "name",
										Type:            tftypes.String,
										Required:        true,
										Description:     "Rule name.",
										DescriptionKind: tfprotov6.StringKindPlain,
									},
								},
							},
						},
					},
				},
			},
			// List-nested block with MaxItems=2: more entries must fail.
			"fx_max_list_block": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					BlockTypes: []*tfprotov6.SchemaNestedBlock{
						{
							TypeName: "rule",
							Nesting:  tfprotov6.SchemaNestedBlockNestingModeList,
							MaxItems: 2,
							Block: &tfprotov6.SchemaBlock{
								Attributes: []*tfprotov6.SchemaAttribute{
									{
										Name:            "name",
										Type:            tftypes.String,
										Required:        true,
										Description:     "Rule name.",
										DescriptionKind: tfprotov6.StringKindPlain,
									},
								},
							},
						},
					},
				},
			},
			// List-nested block: zero or more `rule { … }`.
			"fx_list_block": {
				Version: 0,
				Block: &tfprotov6.SchemaBlock{
					BlockTypes: []*tfprotov6.SchemaNestedBlock{
						{
							TypeName: "rule",
							Nesting:  tfprotov6.SchemaNestedBlockNestingModeList,
							Block: &tfprotov6.SchemaBlock{
								Attributes: []*tfprotov6.SchemaAttribute{
									{
										Name:            "name",
										Type:            tftypes.String,
										Required:        true,
										Description:     "Rule name.",
										DescriptionKind: tfprotov6.StringKindPlain,
									},
								},
							},
						},
					},
				},
			},
		},
		DataSourceSchemas: map[string]*tfprotov6.Schema{},
	}, nil
}

func (p *fixtureProvider) ValidateProviderConfig(context.Context, *tfprotov6.ValidateProviderConfigRequest) (*tfprotov6.ValidateProviderConfigResponse, error) {
	return &tfprotov6.ValidateProviderConfigResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ValidateProviderConfig")},
	}, nil
}

// ConfigureProvider is a no-op: the fixture takes no provider-level
// configuration. It must succeed so real clients can proceed to
// per-resource RPCs.
func (p *fixtureProvider) ConfigureProvider(context.Context, *tfprotov6.ConfigureProviderRequest) (*tfprotov6.ConfigureProviderResponse, error) {
	return &tfprotov6.ConfigureProviderResponse{}, nil
}

// StopProvider must succeed even when other methods return diagnostics:
// clients call it during orderly shutdown regardless of prior errors.
func (p *fixtureProvider) StopProvider(context.Context, *tfprotov6.StopProviderRequest) (*tfprotov6.StopProviderResponse, error) {
	return &tfprotov6.StopProviderResponse{}, nil
}

func (p *fixtureProvider) GetResourceIdentitySchemas(context.Context, *tfprotov6.GetResourceIdentitySchemasRequest) (*tfprotov6.GetResourceIdentitySchemasResponse, error) {
	return &tfprotov6.GetResourceIdentitySchemasResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("GetResourceIdentitySchemas")},
	}, nil
}

// --- StateStoreServer ---

func (p *fixtureProvider) ValidateStateStoreConfig(context.Context, *tfprotov6.ValidateStateStoreConfigRequest) (*tfprotov6.ValidateStateStoreConfigResponse, error) {
	return &tfprotov6.ValidateStateStoreConfigResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ValidateStateStoreConfig")},
	}, nil
}

func (p *fixtureProvider) ConfigureStateStore(context.Context, *tfprotov6.ConfigureStateStoreRequest) (*tfprotov6.ConfigureStateStoreResponse, error) {
	return &tfprotov6.ConfigureStateStoreResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ConfigureStateStore")},
	}, nil
}

func (p *fixtureProvider) ReadStateBytes(context.Context, *tfprotov6.ReadStateBytesRequest) (*tfprotov6.ReadStateBytesStream, error) {
	return &tfprotov6.ReadStateBytesStream{
		Chunks: func(yield func(tfprotov6.ReadStateByteChunk) bool) {
			yield(tfprotov6.ReadStateByteChunk{
				Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ReadStateBytes")},
			})
		},
	}, nil
}

func (p *fixtureProvider) WriteStateBytes(context.Context, *tfprotov6.WriteStateBytesStream) (*tfprotov6.WriteStateBytesResponse, error) {
	return &tfprotov6.WriteStateBytesResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("WriteStateBytes")},
	}, nil
}

func (p *fixtureProvider) GetStates(context.Context, *tfprotov6.GetStatesRequest) (*tfprotov6.GetStatesResponse, error) {
	return &tfprotov6.GetStatesResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("GetStates")},
	}, nil
}

func (p *fixtureProvider) DeleteState(context.Context, *tfprotov6.DeleteStateRequest) (*tfprotov6.DeleteStateResponse, error) {
	return &tfprotov6.DeleteStateResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("DeleteState")},
	}, nil
}

// --- ResourceServer ---

// ValidateResourceConfig usually succeeds. fx_check_write_only_capability
// requires the caller to have announced
// `ClientCapabilities.WriteOnlyAttributesAllowed`; the SDK exposes that
// capability on this RPC in v6, so it's where the check has to live.
func (p *fixtureProvider) ValidateResourceConfig(_ context.Context, req *tfprotov6.ValidateResourceConfigRequest) (*tfprotov6.ValidateResourceConfigResponse, error) {
	if req.TypeName == "fx_check_write_only_capability" {
		if req.ClientCapabilities == nil || !req.ClientCapabilities.WriteOnlyAttributesAllowed {
			return &tfprotov6.ValidateResourceConfigResponse{
				Diagnostics: []*tfprotov6.Diagnostic{{
					Severity: tfprotov6.DiagnosticSeverityError,
					Summary:  "client did not announce write_only_attributes_allowed",
				}},
			}, nil
		}
	}
	return &tfprotov6.ValidateResourceConfigResponse{}, nil
}

func (p *fixtureProvider) UpgradeResourceState(context.Context, *tfprotov6.UpgradeResourceStateRequest) (*tfprotov6.UpgradeResourceStateResponse, error) {
	return &tfprotov6.UpgradeResourceStateResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("UpgradeResourceState")},
	}, nil
}

func (p *fixtureProvider) ReadResource(context.Context, *tfprotov6.ReadResourceRequest) (*tfprotov6.ReadResourceResponse, error) {
	return &tfprotov6.ReadResourceResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ReadResource")},
	}, nil
}

// PlanResourceChange usually echoes the proposed state. Resource types
// that have computed attributes fill those in here so the projection
// onto declared outputs on the nixops4 side has something to pick up.
// Apply then just echoes what Plan settled on.
func (p *fixtureProvider) PlanResourceChange(_ context.Context, req *tfprotov6.PlanResourceChangeRequest) (*tfprotov6.PlanResourceChangeResponse, error) {
	// fx_die: exit here if the caller asked for a Plan-time crash.
	if req.TypeName == "fx_die" {
		if p, ok := parseDieParams(req.ProposedNewState); ok && p.CrashingOperation == "plan" {
			os.Exit(p.ExitCode)
		}
	}
	// fx_hang_on_plan: block forever so the adapter's per-RPC
	// timeout can be exercised. The child is reaped on shutdown.
	if req.TypeName == "fx_hang_on_plan" {
		select {}
	}
	// fx_defer: return a Deferred Plan response if the caller asked.
	if req.TypeName == "fx_defer" {
		if p, ok := parseDeferParams(req.ProposedNewState); ok && p.DeferringOperation == "plan" {
			return &tfprotov6.PlanResourceChangeResponse{
				PlannedState: req.ProposedNewState,
				Deferred:     &tfprotov6.Deferred{Reason: parseDeferReason(p.Reason)},
			}, nil
		}
	}
	// Assert that the adapter honours write_only on prior_state.
	if req.TypeName == "fx_write_only" && priorStateHasField(req.PriorState, "secret") {
		return &tfprotov6.PlanResourceChangeResponse{
			Diagnostics: []*tfprotov6.Diagnostic{{
				Severity: tfprotov6.DiagnosticSeverityError,
				Summary:  "prior_state.secret leaked",
				Detail:   "write_only attribute must be stripped from prior_state by the adapter",
			}},
		}, nil
	}
	planned := req.ProposedNewState
	switch req.TypeName {
	case "fx_computed":
		enriched, err := planFxComputed(req.ProposedNewState)
		if err != nil {
			return &tfprotov6.PlanResourceChangeResponse{
				Diagnostics: []*tfprotov6.Diagnostic{{
					Severity: tfprotov6.DiagnosticSeverityError,
					Summary:  "planFxComputed",
					Detail:   err.Error(),
				}},
			}, nil
		}
		planned = enriched
	case "fx_computed_block_attr":
		enriched, err := planFxComputedBlockAttr(req.ProposedNewState)
		if err != nil {
			return &tfprotov6.PlanResourceChangeResponse{
				Diagnostics: []*tfprotov6.Diagnostic{{
					Severity: tfprotov6.DiagnosticSeverityError,
					Summary:  "planFxComputedBlockAttr",
					Detail:   err.Error(),
				}},
			}, nil
		}
		planned = enriched
	}
	return &tfprotov6.PlanResourceChangeResponse{
		PlannedState: planned,
	}, nil
}

// dieParams controls the fx_die fixture: which RPC exits, and with
// what status. The fields are wire-visible attributes on the schema.
type dieParams struct {
	CrashingOperation string `json:"crashing_operation"`
	ExitCode          int    `json:"exit_code"`
}

// parseDieParams reads the fx_die attributes from a DynamicValue's
// JSON payload. Malformed input returns ok=false so the caller
// treats it as "no crash requested" rather than crashing with an
// undefined status.
func parseDieParams(pns *tfprotov6.DynamicValue) (dieParams, bool) {
	if pns == nil || len(pns.JSON) == 0 {
		return dieParams{}, false
	}
	var p dieParams
	if err := json.Unmarshal(pns.JSON, &p); err != nil {
		return dieParams{}, false
	}
	if p.CrashingOperation == "" {
		return dieParams{}, false
	}
	return p, true
}

// deferParams controls the fx_defer fixture: which RPC returns
// Deferred, and with what reason.
type deferParams struct {
	DeferringOperation string `json:"deferring_operation"`
	Reason             string `json:"reason"`
}

func parseDeferParams(pns *tfprotov6.DynamicValue) (deferParams, bool) {
	if pns == nil || len(pns.JSON) == 0 {
		return deferParams{}, false
	}
	var p deferParams
	if err := json.Unmarshal(pns.JSON, &p); err != nil {
		return deferParams{}, false
	}
	if p.DeferringOperation == "" {
		return deferParams{}, false
	}
	return p, true
}

func parseDeferReason(s string) tfprotov6.DeferredReason {
	switch s {
	case "RESOURCE_CONFIG_UNKNOWN":
		return tfprotov6.DeferredReasonResourceConfigUnknown
	case "PROVIDER_CONFIG_UNKNOWN":
		return tfprotov6.DeferredReasonProviderConfigUnknown
	case "ABSENT_PREREQ":
		return tfprotov6.DeferredReasonAbsentPrereq
	}
	return tfprotov6.DeferredReasonUnknown
}

// priorStateHasField reports whether the (JSON) prior_state contains
// the named top-level key. Used by fixture asserts against the
// adapter's stripping of write_only attributes.
func priorStateHasField(pns *tfprotov6.DynamicValue, key string) bool {
	if pns == nil || len(pns.JSON) == 0 {
		return false
	}
	var m map[string]any
	if err := json.Unmarshal(pns.JSON, &m); err != nil {
		return false
	}
	_, ok := m[key]
	return ok
}

// planFxComputed injects a synthetic `id` into the planned state. On
// destroy the proposed state is a JSON null; leave it untouched so the
// existing destroy path (echo null → new_state null) still works.
func planFxComputed(pns *tfprotov6.DynamicValue) (*tfprotov6.DynamicValue, error) {
	if pns == nil || len(pns.JSON) == 0 {
		return pns, nil
	}
	var m map[string]any
	if err := json.Unmarshal(pns.JSON, &m); err != nil {
		return nil, err
	}
	if m == nil {
		return pns, nil
	}
	value, _ := m["value"].(string)
	m["id"] = "computed-" + value
	out, err := json.Marshal(m)
	if err != nil {
		return nil, err
	}
	return &tfprotov6.DynamicValue{JSON: out}, nil
}

// planFxComputedBlockAttr populates the computed `token` inside the
// nested `settings` block. Exercises new_state projection through a
// Single-nested block. When the user omits `settings`, leave the
// block null so the destroy path stays untouched.
func planFxComputedBlockAttr(pns *tfprotov6.DynamicValue) (*tfprotov6.DynamicValue, error) {
	if pns == nil || len(pns.JSON) == 0 {
		return pns, nil
	}
	var m map[string]any
	if err := json.Unmarshal(pns.JSON, &m); err != nil {
		return nil, err
	}
	if m == nil {
		return pns, nil
	}
	if settings, ok := m["settings"].(map[string]any); ok && settings != nil {
		endpoint, _ := settings["endpoint"].(string)
		settings["token"] = "token-for-" + endpoint
	}
	out, err := json.Marshal(m)
	if err != nil {
		return nil, err
	}
	return &tfprotov6.DynamicValue{JSON: out}, nil
}

// ApplyResourceChange echoes the planned state as the new state.
// This is the minimum a create/update needs to do.
func (p *fixtureProvider) ApplyResourceChange(_ context.Context, req *tfprotov6.ApplyResourceChangeRequest) (*tfprotov6.ApplyResourceChangeResponse, error) {
	// fx_die: exit here if the caller asked for an Apply-time crash.
	if req.TypeName == "fx_die" {
		if p, ok := parseDieParams(req.PlannedState); ok && p.CrashingOperation == "apply" {
			os.Exit(p.ExitCode)
		}
	}
	return &tfprotov6.ApplyResourceChangeResponse{
		NewState: req.PlannedState,
	}, nil
}

func (p *fixtureProvider) ImportResourceState(context.Context, *tfprotov6.ImportResourceStateRequest) (*tfprotov6.ImportResourceStateResponse, error) {
	return &tfprotov6.ImportResourceStateResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ImportResourceState")},
	}, nil
}

func (p *fixtureProvider) MoveResourceState(context.Context, *tfprotov6.MoveResourceStateRequest) (*tfprotov6.MoveResourceStateResponse, error) {
	return &tfprotov6.MoveResourceStateResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("MoveResourceState")},
	}, nil
}

func (p *fixtureProvider) UpgradeResourceIdentity(context.Context, *tfprotov6.UpgradeResourceIdentityRequest) (*tfprotov6.UpgradeResourceIdentityResponse, error) {
	return &tfprotov6.UpgradeResourceIdentityResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("UpgradeResourceIdentity")},
	}, nil
}

func (p *fixtureProvider) GenerateResourceConfig(context.Context, *tfprotov6.GenerateResourceConfigRequest) (*tfprotov6.GenerateResourceConfigResponse, error) {
	return &tfprotov6.GenerateResourceConfigResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("GenerateResourceConfig")},
	}, nil
}

// --- DataSourceServer ---

func (p *fixtureProvider) ValidateDataResourceConfig(context.Context, *tfprotov6.ValidateDataResourceConfigRequest) (*tfprotov6.ValidateDataResourceConfigResponse, error) {
	return &tfprotov6.ValidateDataResourceConfigResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ValidateDataResourceConfig")},
	}, nil
}

func (p *fixtureProvider) ReadDataSource(context.Context, *tfprotov6.ReadDataSourceRequest) (*tfprotov6.ReadDataSourceResponse, error) {
	return &tfprotov6.ReadDataSourceResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ReadDataSource")},
	}, nil
}

// --- FunctionServer ---

func (p *fixtureProvider) GetFunctions(context.Context, *tfprotov6.GetFunctionsRequest) (*tfprotov6.GetFunctionsResponse, error) {
	return &tfprotov6.GetFunctionsResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("GetFunctions")},
	}, nil
}

func (p *fixtureProvider) CallFunction(context.Context, *tfprotov6.CallFunctionRequest) (*tfprotov6.CallFunctionResponse, error) {
	return &tfprotov6.CallFunctionResponse{
		Error: &tfprotov6.FunctionError{Text: "nixops4-tfproto fixture: CallFunction not implemented"},
	}, nil
}

// --- EphemeralResourceServer ---

func (p *fixtureProvider) ValidateEphemeralResourceConfig(context.Context, *tfprotov6.ValidateEphemeralResourceConfigRequest) (*tfprotov6.ValidateEphemeralResourceConfigResponse, error) {
	return &tfprotov6.ValidateEphemeralResourceConfigResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ValidateEphemeralResourceConfig")},
	}, nil
}

func (p *fixtureProvider) OpenEphemeralResource(context.Context, *tfprotov6.OpenEphemeralResourceRequest) (*tfprotov6.OpenEphemeralResourceResponse, error) {
	return &tfprotov6.OpenEphemeralResourceResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("OpenEphemeralResource")},
	}, nil
}

func (p *fixtureProvider) RenewEphemeralResource(context.Context, *tfprotov6.RenewEphemeralResourceRequest) (*tfprotov6.RenewEphemeralResourceResponse, error) {
	return &tfprotov6.RenewEphemeralResourceResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("RenewEphemeralResource")},
	}, nil
}

func (p *fixtureProvider) CloseEphemeralResource(context.Context, *tfprotov6.CloseEphemeralResourceRequest) (*tfprotov6.CloseEphemeralResourceResponse, error) {
	return &tfprotov6.CloseEphemeralResourceResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("CloseEphemeralResource")},
	}, nil
}

// --- ListResourceServer ---

func (p *fixtureProvider) ValidateListResourceConfig(context.Context, *tfprotov6.ValidateListResourceConfigRequest) (*tfprotov6.ValidateListResourceConfigResponse, error) {
	return &tfprotov6.ValidateListResourceConfigResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ValidateListResourceConfig")},
	}, nil
}

func (p *fixtureProvider) ListResource(context.Context, *tfprotov6.ListResourceRequest) *tfprotov6.ListResourceServerStream {
	return &tfprotov6.ListResourceServerStream{
		Results: func(yield func(tfprotov6.ListResourceResult) bool) {
			yield(tfprotov6.ListResourceResult{
				Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ListResource")},
			})
		},
	}
}

// --- ActionServer ---

func (p *fixtureProvider) ValidateActionConfig(context.Context, *tfprotov6.ValidateActionConfigRequest) (*tfprotov6.ValidateActionConfigResponse, error) {
	return &tfprotov6.ValidateActionConfigResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("ValidateActionConfig")},
	}, nil
}

func (p *fixtureProvider) PlanAction(context.Context, *tfprotov6.PlanActionRequest) (*tfprotov6.PlanActionResponse, error) {
	return &tfprotov6.PlanActionResponse{
		Diagnostics: []*tfprotov6.Diagnostic{notImplemented("PlanAction")},
	}, nil
}

func (p *fixtureProvider) InvokeAction(context.Context, *tfprotov6.InvokeActionRequest) *tfprotov6.InvokeActionServerStream {
	return &tfprotov6.InvokeActionServerStream{
		Events: func(yield func(tfprotov6.InvokeActionEvent) bool) {
			yield(tfprotov6.InvokeActionEvent{
				Type: tfprotov6.CompletedInvokeActionEventType{
					Diagnostics: []*tfprotov6.Diagnostic{notImplemented("InvokeAction")},
				},
			})
		},
	}
}
