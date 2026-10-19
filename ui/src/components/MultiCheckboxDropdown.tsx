import { ChevronDown, Funnel, Layers3, Search } from 'lucide-react';
import { useState } from 'react';

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
	const summary = selected.length ? selected[0].label : (props.placeholder ?? 'Select');
	const hasExplicitSelection = selected.length > 0 && summary !== props.placeholder;
	const extraCount = selected.length > 1 ? selected.length - 1 : 0;
	const Icon = props.kind === 'group' ? Layers3 : Funnel;

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
					hasExplicitSelection
						? 'button multi-check-trigger has-selection'
						: 'button multi-check-trigger'
				}
				type="button"
				aria-expanded={open}
				onClick={() => toggleOpen(!open)}
			>
				<span className="multi-check-trigger-main">
					<Icon size={14} />
					<span>{summary}</span>
				</span>
				{extraCount ? <span className="multi-check-count">+{extraCount}</span> : null}
				<ChevronDown size={15} />
			</button>
			{open ? (
				<div className="multi-check-popover">
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
								<span>{option.label}</span>
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
