import { providerLabel } from "../../config";
import {
  isWildcardModelName,
  selectedConfiguredModelName,
  wildcardModelPrefix,
} from "../../modelResolution";
import type { LlmModel, LlmVirtualModel, ProviderName } from "../../types";
import { ProviderIcon } from "../../components/ProviderIcon";

export function modelTargetOptions(models: LlmModel[]) {
  return models.map((model) => ({
    value: model.name,
    label: model.name,
    icon: (
      <ProviderIcon provider={providerLabel(model.provider) as ProviderName} />
    ),
    searchText: `${model.name} ${providerLabel(model.provider)}`,
  }));
}

export function defaultVirtualTargetModel(models: LlmModel[]) {
  const model = models[0];
  if (!model) return "";
  return isWildcardModelName(model.name)
    ? wildcardModelPrefix(model.name)
    : model.name;
}

export function isIncompleteWildcardTarget(
  targetModel: string,
  baseModels: LlmModel[],
) {
  const selected = selectedConfiguredModelName(targetModel, baseModels);
  if (!selected || !isWildcardModelName(selected)) return false;
  return (
    targetModel === selected || targetModel === wildcardModelPrefix(selected)
  );
}

export function failoverTargetGroups(
  targets: NonNullable<LlmVirtualModel["routing"]["failover"]>["targets"],
) {
  const priorities = [
    ...new Set(targets.map((target) => target.priority ?? 0)),
  ].sort((left, right) => left - right);
  return priorities.map((priority) =>
    targets.filter((target) => (target.priority ?? 0) === priority),
  );
}

export function virtualModelStrategy(model: LlmVirtualModel) {
  if (model.routing.conditional) return "conditional";
  if (model.routing.failover) return "failover";
  return "weighted";
}

export function virtualModelSummary(model: LlmVirtualModel) {
  if (model.routing.conditional) {
    const targets = model.routing.conditional.targets ?? [];
    const rules = targets.filter((target) => target.when?.trim()).length;
    const hasFallback = targets.some((target) => !target.when?.trim());
    return `${rules} ${rules === 1 ? "rule" : "rules"}${hasFallback ? ", fallback" : ""}`;
  }
  if (model.routing.failover) {
    const targets = model.routing.failover.targets ?? [];
    const priorities = new Set(targets.map((target) => target.priority)).size;
    return `${priorities} ${priorities === 1 ? "priority" : "priorities"}, ${targets.length} ${targets.length === 1 ? "target" : "targets"}`;
  }
  const targets = model.routing.weighted?.targets ?? [];
  return `${targets.length} weighted ${targets.length === 1 ? "target" : "targets"}`;
}
