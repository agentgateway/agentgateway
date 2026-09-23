import type { DumpModel } from '@/gateway-admin';

/**
 * Model names a client can request: public concrete models and virtual models.
 * Internal models are excluded because they can only be targeted by virtual models.
 * The same model can be served by several listeners, so names are deduplicated.
 */
export function requestableDumpModelNames(models: DumpModel[]) {
	const names = new Set<string>();
	for (const model of models) {
		if ('concrete' in model.kind) {
			if (model.kind.concrete.visibility === 'public') names.add(model.name);
		} else {
			names.add(model.name);
		}
	}
	return [...names].sort((left, right) => left.localeCompare(right));
}
