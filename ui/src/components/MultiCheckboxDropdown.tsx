import { Checkbox, Select } from 'antd';
import { ChevronDown, Funnel, Layers3 } from 'lucide-react';

import { selectComponents } from '@/components/Primitives';

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
	 * Lets the user type to narrow the list of values to pick from. The text never becomes part of
	 * the query, so the filter changes only when a value is picked.
	 */
	searchLabel?: string;
	onChange: (values: string[]) => void;
}) {
	const Icon = props.kind === 'group' ? Layers3 : Funnel;
	return (
		<div className="multi-check">
			<span>{props.label}</span>
			<Select
				components={selectComponents}
				mode="multiple"
				aria-label={props.label}
				prefix={<Icon size={14} />}
				suffixIcon={<ChevronDown size={15} />}
				value={props.values}
				options={props.options}
				placeholder={props.allLabel ?? props.placeholder ?? 'Select'}
				maxTagCount="responsive"
				maxTagPlaceholder={omitted => (
					<span title={omitted.map(option => option.label).join(', ')}>+{omitted.length}</span>
				)}
				allowClear
				popupMatchSelectWidth={false}
				classNames={{ popup: { root: 'multi-check-menu' } }}
				styles={{ popup: { root: { minWidth: 240 } } }}
				listHeight={262}
				transitionName=""
				menuItemSelectedIcon={null}
				optionRender={option => (
					<span className="multi-check-option">
						<Checkbox checked={props.values.includes(String(option.value))} tabIndex={-1} />
						<span title={String(option.label)}>{option.label}</span>
					</span>
				)}
				popupRender={menu => (
					<>
						{/* Stays visible when nothing matches, so a selection can always be cleared. */}
						{props.allLabel ? (
							<button
								type="button"
								className="multi-check-option all"
								tabIndex={-1}
								onMouseDown={event => event.preventDefault()}
								onClick={() => props.onChange([])}
							>
								<Checkbox checked={props.values.length === 0} tabIndex={-1} />
								<span>{props.allLabel}</span>
							</button>
						) : null}
						{menu}
					</>
				)}
				showSearch={
					props.searchLabel
						? {
								filterOption: (input, option) =>
									matchRank(String(option?.label ?? ''), input.trim().toLowerCase()) >= 0,
								filterSort: (left, right, info) => {
									const term = info.searchValue.trim().toLowerCase();
									return (
										matchRank(String(left.label), term) - matchRank(String(right.label), term) ||
										String(left.label).localeCompare(String(right.label))
									);
								}
							}
						: false
				}
				notFoundContent={props.options.length ? 'No matching values.' : 'No values found.'}
				onChange={props.onChange}
			/>
		</div>
	);
}
