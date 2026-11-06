/**
 * How many leading chips to show in `available` px. Space for the overflow count is only reserved
 * when something overflows, and the first chip is always shown (it truncates if it has to).
 */
export function fitChipCount(
	chipWidths: number[],
	countWidth: number,
	available: number,
	gap: number
) {
	const total = chipWidths.reduce((sum, width) => sum + width + gap, -gap);
	if (total <= available) return chipWidths.length;

	let used = countWidth;
	let fit = 0;
	for (const width of chipWidths) {
		used += width + gap;
		if (used > available) break;
		fit += 1;
	}
	return Math.max(fit, Math.min(chipWidths.length, 1));
}
