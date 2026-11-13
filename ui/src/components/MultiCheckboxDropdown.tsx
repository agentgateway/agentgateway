import { Select } from 'antd';
import { Funnel, Layers3 } from 'lucide-react';

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
				mode="multiple"
				aria-label={props.label}
				prefix={<Icon size={14} />}
				value={props.values}
				options={props.options}
				placeholder={props.allLabel ?? props.placeholder ?? 'Select'}
				maxTagCount={1}
				allowClear
				popupMatchSelectWidth={false}
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
