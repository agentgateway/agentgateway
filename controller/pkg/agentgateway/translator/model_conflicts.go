package translator

import (
	"cmp"
	"fmt"
	"slices"
	"strings"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// modelRouterKey is one model router. A listener attachment uses listener.
// An HTTPRoute model rule uses httpRoute, shared by every listener that rule attaches to.
type modelRouterKey struct {
	listener  string
	httpRoute string
}

type modelClaim struct {
	model  *agentgateway.AgentgatewayModel
	router modelRouterKey
}

// acceptedModelParents resolves model parents and drops routers where a newer
// duplicate match.model loses to an older model. Wildcard patterns are left alone.
func acceptedModelParents(ctx RouteContext, model *agentgateway.AgentgatewayModel) []RouteParentReference {
	return rejectDuplicateModelNames(ctx, model, extractModelParentReferenceInfo(ctx, model))
}

func rejectDuplicateModelNames(ctx RouteContext, model *agentgateway.AgentgatewayModel, parents []RouteParentReference) []RouteParentReference {
	if modelMatchIsWildcard(effectiveModelName(model)) {
		return parents
	}
	claims := modelClaimsFor(model, parents)
	if len(claims) == 0 {
		return parents
	}
	name := effectiveModelName(model)
	for _, other := range allModels(ctx) {
		if sameModel(other, model) {
			continue
		}
		otherName := effectiveModelName(other)
		if otherName != name || modelMatchIsWildcard(otherName) {
			continue
		}
		for _, parent := range extractModelParentReferenceInfo(ctx, other) {
			router, ok := modelRouterKeyOf(parent)
			if !ok {
				continue
			}
			claims = append(claims, modelClaim{model: other, router: router})
		}
	}
	return denyConflictingModelParents(model, parents, claims)
}

func denyConflictingModelParents(model *agentgateway.AgentgatewayModel, parents []RouteParentReference, claims []modelClaim) []RouteParentReference {
	losers := losingModelRouters(model, claims)
	if len(losers) == 0 {
		return parents
	}
	name := effectiveModelName(model)
	cloned := slices.Clone(parents)
	for i, parent := range cloned {
		router, ok := modelRouterKeyOf(parent)
		if !ok {
			continue
		}
		winner, lost := losers[router]
		if !lost {
			continue
		}
		cloned[i].DeniedReason = &ParentError{
			Reason:  ParentErrorModelConflict,
			Message: modelNameConflictMessage(name, winner, parent),
		}
	}
	return cloned
}

// losingModelRouters returns routers where model is not the oldest exact match.model.
// The oldest object wins: creationTimestamp, then namespace/name, then GroupVersionKind.
func losingModelRouters(model *agentgateway.AgentgatewayModel, claims []modelClaim) map[modelRouterKey]*agentgateway.AgentgatewayModel {
	name := effectiveModelName(model)
	if modelMatchIsWildcard(name) {
		return nil
	}
	winners := map[modelRouterKey]*agentgateway.AgentgatewayModel{}
	for _, claim := range claims {
		if !sameModel(claim.model, model) {
			continue
		}
		if _, seen := winners[claim.router]; !seen {
			winners[claim.router] = model
		}
	}
	if len(winners) == 0 {
		return nil
	}
	for _, claim := range claims {
		if sameModel(claim.model, model) {
			continue
		}
		winner, ok := winners[claim.router]
		if !ok {
			continue
		}
		claimName := effectiveModelName(claim.model)
		if claimName != name || modelMatchIsWildcard(claimName) {
			continue
		}
		if modelPrecedes(claim.model, winner) {
			winners[claim.router] = claim.model
		}
	}
	var losers map[modelRouterKey]*agentgateway.AgentgatewayModel
	for router, winner := range winners {
		if sameModel(winner, model) {
			continue
		}
		if losers == nil {
			losers = map[modelRouterKey]*agentgateway.AgentgatewayModel{}
		}
		losers[router] = winner
	}
	return losers
}

func modelClaimsFor(model *agentgateway.AgentgatewayModel, parents []RouteParentReference) []modelClaim {
	claims := make([]modelClaim, 0, len(parents))
	for _, parent := range parents {
		router, ok := modelRouterKeyOf(parent)
		if !ok {
			continue
		}
		claims = append(claims, modelClaim{model: model, router: router})
	}
	return claims
}

func modelRouterKeyOf(parent RouteParentReference) (modelRouterKey, bool) {
	if parent.DeniedReason != nil {
		return modelRouterKey{}, false
	}
	if parent.ModelRouterKey != "" {
		return modelRouterKey{httpRoute: parent.ModelRouterKey}, true
	}
	if parent.ListenerKey == "" {
		return modelRouterKey{}, false
	}
	return modelRouterKey{listener: parent.ListenerKey}, true
}

func modelPrecedes(a, b *agentgateway.AgentgatewayModel) bool {
	if c := a.CreationTimestamp.Compare(b.CreationTimestamp.Time); c != 0 {
		return c < 0
	}
	if c := cmp.Compare(a.Namespace, b.Namespace); c != 0 {
		return c < 0
	}
	if c := cmp.Compare(a.Name, b.Name); c != 0 {
		return c < 0
	}
	ag, bg := modelGVK(a), modelGVK(b)
	if c := cmp.Compare(ag.Group, bg.Group); c != 0 {
		return c < 0
	}
	if c := cmp.Compare(ag.Version, bg.Version); c != 0 {
		return c < 0
	}
	return ag.Kind < bg.Kind
}

func sameModel(a, b *agentgateway.AgentgatewayModel) bool {
	return a.Namespace == b.Namespace && a.Name == b.Name && modelGVK(a) == modelGVK(b)
}

func modelMatchIsWildcard(name string) bool {
	return strings.Contains(name, "*")
}

func modelConflictIdentity(model *agentgateway.AgentgatewayModel) string {
	id := model.Namespace + "/" + model.Name
	if modelGVK(model) == wellknown.AgentgatewayModelGVK {
		return id
	}
	return modelGVK(model).Kind + " " + id
}

func modelNameConflictMessage(name string, winner *agentgateway.AgentgatewayModel, parent RouteParentReference) string {
	who := modelConflictIdentity(winner)
	if parent.ModelRouterKey != "" {
		where := fmt.Sprintf("HTTPRoute %s/%s", parent.ParentKey.Namespace, parent.ParentKey.Name)
		if parent.ParentSection != "" {
			where += " rule " + string(parent.ParentSection)
		}
		return fmt.Sprintf("match.model %q conflicts with %s on %s", name, who, where)
	}
	return fmt.Sprintf("match.model %q conflicts with %s on listener %s", name, who, parent.ListenerKey)
}
