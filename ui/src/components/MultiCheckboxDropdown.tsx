import { ChevronDown, Funnel, Layers3, Search } from 'lucide-react';
import { useState } from 'react';

import { useDismissiblePopover } from '@/components/Primitives';

/**
 * Ranks a label against the typed term so the closest matches are read first: an exact value,
 * then values that start with the term, then values that only contain it. Values that do not
 * match are dropped.
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
	/** Adds a free-text box to the popover that narrows the options and is reported back to the page. */
	searchLabel?: string;
	searchText?: string;
	onSearchChange?: (value: string) => void;
	onChange: (values: string[]) => void;
}) {
	const [open, setOpen] = useState(false);
	const ref = useDismissiblePopover<HTMLDivElement>(open, () => setOpen(false));
	const search = props.onSearchChange ? (props.searchText ?? '').trim().toLowerCase() : '';
	const shownOptions = search
		? props.options
				.map(option => ({ option, rank: matchRank(option.label, search) }))
				.filter(entry => entry.rank >= 0)
				.sort(
					(left, right) =>
						left.rank - right.rank || left.option.label.localeCompare(right.option.label)
				)
				.map(entry => entry.option)
		: props.options;
	// The summary must not depend on the typed term, or a search could hide the current selection.
	const selected = props.options.filter(option => props.values.includes(option.value));
	const summary = selected.length ? selected[0].label : (props.placeholder ?? 'Select');
	const hasExplicitSelection = selected.length > 0 && summary !== props.placeholder;
	const extraCount = selected.length > 1 ? selected.length - 1 : 0;
	const Icon = props.kind === 'group' ? Layers3 : Funnel;

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
				onClick={() => setOpen(current => !current)}
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
					{props.onSearchChange ? (
						<label className="multi-check-search">
							<Search size={14} />
							<input
								type="text"
								value={props.searchText ?? ''}
								placeholder={props.searchLabel ?? 'Search'}
								aria-label={props.searchLabel ?? `Search ${props.label}`}
								onChange={event => props.onSearchChange?.(event.target.value)}
							/>
						</label>
					) : null}
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
							{search && props.options.length ? 'No matching values.' : 'No values found.'}
						</p>
					)}
				</div>
			) : null}
		</div>
	);
}
