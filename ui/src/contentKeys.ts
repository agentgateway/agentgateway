// React keys for a list that is rendered but never edited in place: each value
// with the number of equal values before it, so repeats stay unique and a key
// moves with its item instead of its position.
export function contentKeys(values: readonly string[]) {
	const seen = new Map<string, number>();
	return values.map(value => {
		const count = seen.get(value) ?? 0;
		seen.set(value, count + 1);
		return `${count}:${value}`;
	});
}
