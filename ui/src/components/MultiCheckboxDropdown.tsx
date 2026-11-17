import { ChevronDown, Funnel, Layers3, Search } from 'lucide-react';
import { useLayoutEffect, useRef, useState } from 'react';

import { fitChipCount } from '@/components/multiCheckFit';
import { useDismissiblePopover } from '@/components/Primitives';

/**
 * Ranks a label against the typed text so the closest matches are read first: an exact value,
 * then values that start with it, then values that only contain it. Values that do not match are
 * dropped.
 */
function matchRank(label: string, term: string) {
	const value = label.toLowerCase();
	if (value === term) return 0;
	if (value.startsWith(term)) return 1;
	if (value.includes(term)) return 2;
	return -1;
}

function SelectedChips(props: { selected: Array<{ value: string; label: string }> }) {
	const { selected } = props;
	const rowRef = useRef<HTMLSpanElement>(null);
	const measureRef = useRef<HTMLSpanElement>(null);
	const [fit, setFit] = useState(selected.length);

	// Widths come from a hidden copy of every chip, so they don't depend on how many are shown.
	// Callers key this by the selection, so a new selection is measured before it paints.
	useLayoutEffect(() => {
		const row = rowRef.current;
		const measure = measureRef.current;
		if (!row || !measure) return;
		const update = () => {
			const widths = Array.from(measure.children, child => (child as HTMLElement).offsetWidth);
			const countWidth = widths.pop() ?? 0;
			const gap = Number.parseFloat(getComputedStyle(row).columnGap) || 0;
			setFit(fitChipCount(widths, countWidth, row.clientWidth, gap));
		};
		update();
		const observer = new ResizeObserver(update);
		observer.observe(row);
		observer.observe(measure);
		return () => observer.disconnect();
	}, []);

	const shown = selected.slice(0, fit);
	const overflow = selected.length - shown.length;
	return (
		<span className="multi-check-values" ref={rowRef}>
			{shown.map(option => (
				<span className="multi-check-chip" key={option.value}>
					{option.label}
				</span>
			))}
			{overflow ? <span className="multi-check-count">+{overflow}</span> : null}
			<span className="multi-check-measure" aria-hidden="true" ref={measureRef}>
				{selected.map(option => (
					<span className="multi-check-chip" key={option.value}>
						{option.label}
					</span>
				))}
				<span className="multi-check-count">+{selected.length}</span>
			</span>
		</span>
	);
}

export function MultiCheckboxDropdown(props: {
	kind: 'group' | 'filter';
	label: string;
	options: Array<{ value: string; label: string }>;
	values: string[];
	placeholder?: string;
	allLabel?: string;
	/**
	 * Adds a free-text box to the popover. The box only narrows the list of values to pick from; it
	 * never becomes part of the query, so the filter changes only when a value is checked.
	 */
	searchLabel?: string;
	onChange: (values: string[]) => void;
}) {
	const [open, setOpen] = useState(false);
	const [query, setQuery] = useState('');
	const ref = useDismissiblePopover<HTMLDivElement>(open, () => setOpen(false));
	const popoverRef = useRef<HTMLDivElement>(null);
	const filter = query.trim().toLowerCase();
	const shownOptions = filter
		? props.options
				.map(option => ({ option, rank: matchRank(option.label, filter) }))
				.filter(entry => entry.rank >= 0)
				.sort(
					(left, right) =>
						left.rank - right.rank || left.option.label.localeCompare(right.option.label)
				)
				.map(entry => entry.option)
		: props.options;
	const selected = props.options.filter(option => props.values.includes(option.value));
	const Icon = props.kind === 'group' ? Layers3 : Funnel;

	// Sized to the longest option, capped at the field's row and kept inside it. The width is locked
	// on open so the popover doesn't shrink and jump as the search narrows the list.
	useLayoutEffect(() => {
		const popover = popoverRef.current;
		const row = popover?.parentElement?.parentElement;
		if (!open || !popover || !row) return;
		const rowBox = row.getBoundingClientRect();
		popover.style.maxWidth = `${rowBox.width}px`;
		const box = popover.getBoundingClientRect();
		popover.style.width = `${Math.ceil(box.width)}px`;
		const overflow = box.right - rowBox.right;
		if (overflow > 0) popover.style.left = `${-Math.ceil(overflow)}px`;
	}, [open]);

	function toggleOpen(nextOpen: boolean) {
		// Reopening always starts from the full list.
		if (nextOpen) setQuery('');
		setOpen(nextOpen);
	}

	function toggle(value: string) {
		props.onChange(
			props.values.includes(value)
				? props.values.filter(item => item !== value)
				: [...props.values, value]
		);
	}

	return (
		<div className="multi-check" ref={ref}>
			<span>{props.label}</span>
			<button
				className={
					selected.length
						? 'button multi-check-trigger has-selection'
						: 'button multi-check-trigger'
				}
				type="button"
				title={selected.length ? selected.map(option => option.label).join(', ') : undefined}
				aria-expanded={open}
				onClick={() => toggleOpen(!open)}
			>
				<span className="multi-check-trigger-main">
					<Icon size={14} />
					{selected.length ? (
						<SelectedChips
							key={selected.map(option => option.value).join('\n')}
							selected={selected}
						/>
					) : (
						<span>{props.placeholder ?? 'Select'}</span>
					)}
				</span>
				<ChevronDown size={15} />
			</button>
			{open ? (
				<div className="multi-check-popover" ref={popoverRef}>
					{props.searchLabel ? (
						<label className="multi-check-search">
							<Search size={14} />
							<input
								type="text"
								value={query}
								placeholder={props.searchLabel}
								aria-label={props.searchLabel}
								onChange={event => setQuery(event.target.value)}
							/>
						</label>
					) : null}
					{/* Kept visible even when nothing matches, so a selection can always be cleared. */}
					{props.allLabel ? (
						<label className="multi-check-option all">
							<input
								type="checkbox"
								checked={props.values.length === 0}
								onChange={() => props.onChange([])}
							/>
							<span>{props.allLabel}</span>
						</label>
					) : null}
					{shownOptions.length ? (
						shownOptions.map(option => (
							<label className="multi-check-option" key={option.value}>
								<input
									type="checkbox"
									checked={props.values.includes(option.value)}
									onChange={() => toggle(option.value)}
								/>
								<span title={option.label}>{option.label}</span>
							</label>
						))
					) : (
						<p className="muted-copy">
							{filter && props.options.length ? 'No matching values.' : 'No values found.'}
						</p>
					)}
				</div>
			) : null}
		</div>
	);
}
