import { AutoComplete, Button, Input } from 'antd';
import { ChevronDown } from 'lucide-react';
import { useState } from 'react';

export function FreeformCombobox(props: {
	id?: string;
	ariaLabel: string;
	'aria-describedby'?: string;
	value: string;
	options: string[];
	onChange: (value: string) => void;
	placeholder?: string;
	emptyText?: string;
}) {
	const [open, setOpen] = useState(false);
	// The chevron lists every option; typing filters by the current text.
	const [browseAll, setBrowseAll] = useState(false);
	const query = browseAll ? '' : props.value.trim().toLowerCase();
	const options = query
		? props.options.filter(option => option.toLowerCase().includes(query))
		: props.options;

	return (
		<AutoComplete
			className="freeform-combobox"
			value={props.value}
			options={options.map(option => ({ value: option, label: option }))}
			open={open}
			onOpenChange={next => {
				setOpen(next);
				if (!next) setBrowseAll(false);
			}}
			onChange={value => {
				setBrowseAll(false);
				props.onChange(value);
			}}
			notFoundContent={props.emptyText ?? 'No matches. Custom values are allowed.'}
		>
			<Input
				id={props.id}
				aria-label={props.ariaLabel}
				aria-describedby={props['aria-describedby']}
				placeholder={props.placeholder}
				autoComplete="off"
				spellCheck={false}
				onFocus={() => setOpen(true)}
				suffix={
					<Button
						type="text"
						size="small"
						aria-label={`Show ${props.ariaLabel} options`}
						icon={<ChevronDown size={15} />}
						onMouseDown={event => {
							event.preventDefault();
							event.stopPropagation();
						}}
						onClick={event => {
							event.stopPropagation();
							// Focus may have opened a filtered list; the chevron switches it to every option.
							setOpen(!(open && browseAll));
							setBrowseAll(true);
						}}
					/>
				}
			/>
		</AutoComplete>
	);
}
