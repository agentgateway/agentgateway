package translator

import (
	"cmp"
	"fmt"
	"slices"
	"strings"

	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// The data plane resolves a requested model name against the models of one router: a Gateway listener
// for directly attached models, or a model-serving HTTPRoute rule. Two models with the same exact name on
// one router would be ambiguous, so exactly one of them is programmed there.
//
// Precedence follows the Gateway API convention: the oldest creationTimestamp wins, then the
// lexicographically smallest namespace/name, then group/kind (only relevant across model kinds).
// Wildcard match.model values are out of scope and keep their existing behavior.

// ModelNameIndex finds the models of every kind that share an exact effective model name. A model's
// translation fetches only the models with its own name, so it is retranslated when one of those changes
// (including their parents), not when unrelated models change.
type ModelNameIndex struct {
	kinds []modelNameKind
}

type modelNameKind struct {
	models krt.Collection[*agentgateway.AgentgatewayModel]
	byName krt.Index[string, *agentgateway.AgentgatewayModel]
}

// NewModelNameIndex indexes the native models and every model kind registered through ModelSources.
func NewModelNameIndex(inputs RouteContextInputs) *ModelNameIndex {
	index := func(models krt.Collection[*agentgateway.AgentgatewayModel]) modelNameKind {
		return modelNameKind{models: models, byName: krt.NewIndex(models, "exact-model-name", func(m *agentgateway.AgentgatewayModel) []string {
			if name, exact := exactModelName(m); exact {
				return []string{name}
			}
			return nil
		})}
	}
	out := &ModelNameIndex{kinds: []modelNameKind{index(inputs.Models)}}
	if inputs.Collections != nil {
		sources := make([]plugins.ModelSource, 0, len(inputs.Collections.ModelSources))
		for _, source := range inputs.Collections.ModelSources {
			sources = append(sources, source)
		}
		slices.SortFunc(sources, func(a, b plugins.ModelSource) int { return strings.Compare(a.GVK.String(), b.GVK.String()) })
		for _, source := range sources {
			out.kinds = append(out.kinds, index(source.Models))
		}
	}
	return out
}

// modelPrecedes reports whether a wins over b. It is a strict total order over distinct models, so the
// result never depends on collection or map iteration order. The full timestamp is compared, not seconds.
func modelPrecedes(a, b *agentgateway.AgentgatewayModel) bool {
	if !a.CreationTimestamp.Equal(&b.CreationTimestamp) {
		return a.CreationTimestamp.Before(&b.CreationTimestamp)
	}
	ga, gb := modelGVK(a), modelGVK(b)
	return cmp.Or(
		strings.Compare(a.Namespace, b.Namespace),
		strings.Compare(a.Name, b.Name),
		strings.Compare(ga.Group, gb.Group),
		strings.Compare(ga.Kind, gb.Kind),
	) < 0
}

func sameModel(a, b *agentgateway.AgentgatewayModel) bool {
	return a.Namespace == b.Namespace && a.Name == b.Name && modelGVK(a).GroupKind() == modelGVK(b).GroupKind()
}

// exactModelName returns the effective model name, excluding wildcard patterns.
func exactModelName(model *agentgateway.AgentgatewayModel) (string, bool) {
	name := effectiveModelName(model)
	return name, !strings.Contains(name, "*")
}

// modelRouterName identifies the router a parent attachment programs. A model attached to an HTTPRoute
// rule joins that rule's router, which is shared across the listeners the rule is attached to; a directly
// attached model joins its listener. The two namespaces cannot collide.
func modelRouterName(parent RouteParentReference) string {
	if parent.ModelRouterKey != "" {
		return "rule:" + parent.ModelRouterKey
	}
	if parent.ListenerKey != "" {
		return "listener:" + parent.ListenerKey
	}
	return ""
}

func modelRouterLabel(parent RouteParentReference) string {
	if parent.ModelRouterKey != "" {
		label := "HTTPRoute " + parent.ParentKey.Namespace + "/" + parent.ParentKey.Name
		if parent.ParentSection != "" {
			label += " rule " + string(parent.ParentSection)
		}
		return label
	}
	return "listener " + parent.ListenerKey
}

// routerWinners returns, per router, the best model with the exact name other than skip. Only attachments
// that pass the existing parent eligibility checks claim a router. Resolving each competitor's parents
// through KRT makes the caller depend on them, so a competitor moving, being renamed or becoming
// ineligible retranslates the caller even though the indexed name key may not change.
func (m *ModelNameIndex) routerWinners(ctx RouteContext, name string, skip *agentgateway.AgentgatewayModel) map[string]*agentgateway.AgentgatewayModel {
	var winners map[string]*agentgateway.AgentgatewayModel
	for _, kind := range m.kinds {
		for _, candidate := range krt.Fetch(ctx.Krt, kind.models, krt.FilterIndex(kind.byName, name)) {
			if skip != nil && sameModel(candidate, skip) {
				continue
			}
			for _, parent := range FilteredReferences(extractModelParentReferenceInfo(ctx, candidate)) {
				router := modelRouterName(parent)
				if router == "" {
					continue
				}
				if winners == nil {
					winners = map[string]*agentgateway.AgentgatewayModel{}
				}
				if best := winners[router]; best == nil || modelPrecedes(candidate, best) {
					winners[router] = candidate
				}
			}
		}
	}
	return winners
}

func modelConflictMessage(model *agentgateway.AgentgatewayModel, name string, winner *agentgateway.AgentgatewayModel, parent RouteParentReference) string {
	label := winner.Namespace + "/" + winner.Name
	if wg, g := modelGVK(winner), modelGVK(model); wg.GroupKind() != g.GroupKind() {
		label = wg.Kind + " " + label
	}
	return fmt.Sprintf("match.model %q conflicts with %s on %s", name, label, modelRouterLabel(parent))
}

// rejectDuplicateModelParents marks attachments that lose an exact model name conflict as denied, so
// they are neither programmed nor counted. Attachments that were already denied keep their reason, and
// the input slice is not modified.
func rejectDuplicateModelParents(ctx RouteContext, model *agentgateway.AgentgatewayModel, parents []RouteParentReference) []RouteParentReference {
	name, exact := exactModelName(model)
	if !exact || ctx.ModelNameIndex == nil || !slices.ContainsFunc(parents, func(p RouteParentReference) bool { return p.DeniedReason == nil }) {
		return parents
	}
	// The model's own eligible parents are already known, so only competitors are resolved.
	competitors := ctx.ModelNameIndex.routerWinners(ctx, name, model)
	var result []RouteParentReference
	for i, parent := range parents {
		if parent.DeniedReason != nil {
			continue
		}
		winner := competitors[modelRouterName(parent)]
		if winner == nil || !modelPrecedes(winner, model) {
			continue
		}
		if result == nil {
			result = slices.Clone(parents)
		}
		result[i].DeniedReason = &ParentError{
			Reason:  ParentErrorConflicted,
			Message: modelConflictMessage(model, name, winner, parent),
		}
	}
	if result == nil {
		return parents
	}
	return result
}

// checkModelTargetServed rejects a typed modelRef target whose exact name is held by a different model on
// the parent's router. The data plane resolves virtual model targets by name, so forwarding the name would
// reach the winner instead of the referenced model.
func checkModelTargetServed(ctx RouteContext, parent RouteParentReference, ref *agentgateway.AgentgatewayModel, name string) error {
	router := modelRouterName(parent)
	if _, exact := exactModelName(ref); !exact || ctx.ModelNameIndex == nil || router == "" {
		return nil
	}
	if winner := ctx.ModelNameIndex.routerWinners(ctx, name, nil)[router]; winner != nil && !sameModel(winner, ref) {
		return fmt.Errorf("model target %s/%s is not served on %s: match.model %q is held by %s/%s",
			ref.Namespace, ref.Name, modelRouterLabel(parent), name, winner.Namespace, winner.Name)
	}
	return nil
}

// modelRouteAttachmentCollection mirrors gatewayRouteAttachmentCollection for models, omitting
// attachments that lost an exact name conflict so attachedRoutes and route-to-gateway
// associations match what is programmed.
func modelRouteAttachmentCollection(
	inputs RouteContextInputs,
	models krt.Collection[*agentgateway.AgentgatewayModel],
	gvk schema.GroupVersionKind,
	opts krtutil.KrtOptions,
) krt.Collection[*plugins.RouteAttachment] {
	return krt.NewManyCollection(models, func(krtctx krt.HandlerContext, obj *agentgateway.AgentgatewayModel) []*plugins.RouteAttachment {
		ctx := inputs.WithCtx(krtctx)
		from := utils.TypedNamespacedName{Kind: gvk.Kind, NamespacedName: config.NamespacedName(obj)}
		refs := FilteredReferences(rejectDuplicateModelParents(ctx, obj, extractParentReferenceInfo(ctx, obj)))
		out := make([]*plugins.RouteAttachment, 0, len(refs))
		for _, e := range refs {
			parentKind := NormalizeReference(e.OriginalReference.Group, e.OriginalReference.Kind, wellknown.GatewayGVK.GroupKind())
			if e.ParentGateway.Name == "" && parentKind != wellknown.ListenerSetGVK.GroupKind() {
				continue
			}
			parent := utils.TypedNamespacedName{Kind: wellknown.GatewayGVK.Kind, NamespacedName: e.ParentGateway}
			switch parentKind {
			case wellknown.GatewayGVK.GroupKind(), wellknown.ServiceGVK.GroupKind(), wellknown.ServiceEntryGVK.GroupKind():
			default:
				parent = e.ParentKey
			}
			out = append(out, &plugins.RouteAttachment{
				From:         from,
				To:           parent,
				Gateway:      e.ParentGateway,
				ListenerName: string(e.ParentSection),
			})
		}
		return out
	}, opts.ToOptions("translator/"+gvk.Kind+"/count")...)
}
