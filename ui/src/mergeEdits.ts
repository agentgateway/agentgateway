// Structured editors load a config value into form state and rebuild the value
// from that state on save, so anything the form does not model would be lost.
// mergeEdits applies only what the user changed back onto the original:
//
//   original  the value the editor was opened with
//   baseline  what the editor outputs before any edit
//   edited    what the editor outputs now
//
// Anything the editor never emitted is kept from `original`; anything it
// emitted and the user changed or cleared is taken from `edited`. `lost` lists
// paths where the editor reshaped the original (so it could not be mapped
// back) and the edit replaces content the editor never showed.
export function mergeEdits(
	original: unknown,
	baseline: unknown,
	edited: unknown
): { value: unknown; lost: string[] } {
	const lost: string[] = [];
	const value = merge(original, baseline, edited, '', lost);
	return { value, lost };
}

function merge(
	original: unknown,
	baseline: unknown,
	edited: unknown,
	path: string,
	lost: string[]
): unknown {
	if (deepEqual(baseline, edited)) return original;
	if (
		isRecord(original) &&
		isRecord(baseline) &&
		isRecord(edited) &&
		keepsAField(original, baseline)
	) {
		const merged: Record<string, unknown> = {};
		for (const key of new Set([...Object.keys(original), ...Object.keys(edited)])) {
			const value =
				edited[key] === undefined
					? baseline[key] === undefined
						? original[key]
						: undefined
					: merge(original[key], baseline[key], edited[key], `${path}.${key}`, lost);
			if (value !== undefined) merged[key] = value;
		}
		return merged;
	}
	if (
		Array.isArray(original) &&
		Array.isArray(baseline) &&
		Array.isArray(edited) &&
		original.length === baseline.length
	) {
		return mergeArray(original, baseline, edited, path, lost);
	}
	if (hasContent(original) && !deepEqual(original, baseline)) lost.push(pathName(path));
	return edited;
}

// Baseline items correspond to original items by index. Edited items that are
// unchanged from some baseline item keep that original item, so removing or
// reordering entries preserves the rest. Rows that look the same but differ in
// hidden fields cannot be told apart by value, so they are matched by position
// below instead. Changed items between two unchanged
// neighbours pair in order with the unused baseline items there when the counts
// match (edits in place), or after the last unchanged item when there are more
// changed items (edits followed by appended entries). Otherwise which entry is
// which is ambiguous, so they are taken as new and the replaced originals are
// reported.
function mergeArray(
	original: unknown[],
	baseline: unknown[],
	edited: unknown[],
	path: string,
	lost: string[]
) {
	const used = new Set<number>();
	const matched = edited.map(item => {
		const candidates = baseline.flatMap((base, i) =>
			!used.has(i) && deepEqual(base, item) ? [i] : []
		);
		if (
			!candidates.length ||
			candidates.some(i => !deepEqual(original[i], original[candidates[0]]))
		)
			return -1;
		used.add(candidates[0]);
		return candidates[0];
	});
	const result: unknown[] = [];
	let run: number[] = [];
	let after = -1;
	const flush = (before: number) => {
		const free = baseline.flatMap((_, j) => (j > after && j < before && !used.has(j) ? [j] : []));
		const appended = before === baseline.length && run.length > free.length;
		if (free.length === run.length || appended) {
			for (const k of run.slice(free.length)) result[k] = edited[k];
			run.slice(0, free.length).forEach((k, n) => {
				result[k] = merge(
					original[free[n]],
					baseline[free[n]],
					edited[k],
					`${path}[${free[n]}]`,
					lost
				);
			});
		} else {
			for (const k of run) result[k] = edited[k];
			if (run.length)
				for (const j of free)
					if (!deepEqual(original[j], baseline[j])) lost.push(pathName(`${path}[${j}]`));
		}
		run = [];
	};
	edited.forEach((_, k) => {
		if (matched[k] < 0) {
			run.push(k);
			return;
		}
		flush(matched[k]);
		result[k] = original[matched[k]];
		after = Math.max(after, matched[k]);
	});
	flush(baseline.length);
	return result;
}

function pathName(path: string) {
	return path.replace(/^\./, '') || '.';
}

export function deepEqual(a: unknown, b: unknown): boolean {
	if (a === b) return true;
	if (Array.isArray(a) && Array.isArray(b))
		return a.length === b.length && a.every((item, i) => deepEqual(item, b[i]));
	if (isRecord(a) && isRecord(b)) {
		const keys = Object.keys(a).filter(key => a[key] !== undefined);
		return (
			keys.length === Object.keys(b).filter(key => b[key] !== undefined).length &&
			keys.every(key => deepEqual(a[key], b[key]))
		);
	}
	return false;
}

// An editor that shows none of a non-empty original's fields in place has
// rewritten it into another shape (e.g. `{backend, path}` or `{host, port, path}`
// shown as `{host: url}`), so the original's fields cannot be kept alongside
// the edit. A field is in place when the baseline has it unchanged, or as the
// same kind of container (its contents are then merged the same way).
function keepsAField(original: Record<string, unknown>, baseline: Record<string, unknown>) {
	const keys = Object.keys(original).filter(key => original[key] !== undefined);
	return (
		!keys.length ||
		!Object.keys(baseline).length ||
		keys.some(
			key =>
				deepEqual(original[key], baseline[key]) ||
				(isRecord(original[key]) && isRecord(baseline[key])) ||
				(Array.isArray(original[key]) && Array.isArray(baseline[key]))
		)
	);
}

function hasContent(value: unknown) {
	return (isRecord(value) || Array.isArray(value)) && Object.keys(value).length > 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}
