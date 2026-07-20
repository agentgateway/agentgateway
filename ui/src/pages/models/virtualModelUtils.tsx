import { providerLabel } from "../../config";
import { tr } from "../../i18n";
import {
  isWildcardModelName,
  resolvedProviderLabel,
  selectedConfiguredModelName,
  wildcardModelPrefix,
} from "../../modelResolution";
import type {
  LlmModel,
  LlmProvider,
  LlmVirtualModel,
  ProviderName,
} from "../../types";
import { ProviderIcon } from "../../components/ProviderIcon";

export function modelTargetOptions(
  models: LlmModel[],
  providers: LlmProvider[],
) {
  return models.map((model) => {
    const provider = resolvedProviderLabel(model.provider, providers);
    return {
      value: model.name,
      label: model.name,
      icon: <ProviderIcon provider={provider as ProviderName} />,
      searchText: `${model.name} ${provider} ${providerLabel(model.provider)}`,
    };
  });
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
  if (model.routing.conditional) return tr("copy.conditional");
  if (model.routing.failover) return tr("copy.failover");
  return tr("copy.weighted");
}

export function virtualModelSummary(model: LlmVirtualModel) {
  if (model.routing.conditional) {
    const targets = model.routing.conditional.targets ?? [];
    const rules = targets.filter((target) => target.when?.trim()).length;
    const hasFallback = targets.some((target) => !target.when?.trim());
    return hasFallback
      ? tr("copy.valueRulesWithFallback", { count: rules })
      : tr("copy.valueRules", { count: rules });
  }
  if (model.routing.failover) {
    const targets = model.routing.failover.targets ?? [];
    const priorities = new Set(targets.map((target) => target.priority)).size;
    return tr("copy.priorityTargetSeparator", [
      tr("copy.priority", { count: priorities }),
      tr("copy.target", { count: targets.length }),
    ]);
  }
  const targets = model.routing.weighted?.targets ?? [];
  return tr("copy.valueWeightedTargets", { count: targets.length });
}
