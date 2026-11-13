import { Button, Input, Tag } from 'antd';
import { X } from 'lucide-react';
import type { ReactNode } from 'react';
import { useState } from 'react';

import { FieldGroup } from '@/components/Primitives';
import { appendUnique } from '@/policies/policyUtils';

export function ListEditor(props: {
	label: string;
	values: string[];
	onChange: (values: string[]) => void;
	placeholder: string;
	emptyText?: string;
	tooltip?: string;
	suggestions?: string[];
	actions?: ReactNode;
}) {
	const [draft, setDraft] = useState('');

	function add(value: string) {
		const next = value.trim();
		if (!next) return;
		props.onChange(appendUnique(props.values, next));
		setDraft('');
	}

	return (
		<FieldGroup label={props.label} tooltip={props.tooltip}>
			<div className="list-editor">
				{props.values.length > 0 ? (
					<div className="chip-list">
						{props.values.map(value => (
							<Tag className="config-chip" key={value}>
								<span>{value}</span>
								<Button
									type="text"
									size="small"
									aria-label={`Remove ${value}`}
									icon={<X size={12} />}
									onClick={() => props.onChange(props.values.filter(item => item !== value))}
								/>
							</Tag>
						))}
					</div>
				) : (
					<div className="empty-inline">{props.emptyText ?? 'No values configured.'}</div>
				)}
				<div className="list-editor-row">
					<Input
						value={draft}
						onChange={event => setDraft(event.target.value)}
						onPressEnter={event => {
							event.preventDefault();
							add(draft);
						}}
						placeholder={props.placeholder}
					/>
					<Button onClick={() => add(draft)}>Add</Button>
					{props.actions}
				</div>
				{props.suggestions?.length ? (
					<div className="suggestion-row">
						{props.suggestions.map(suggestion => (
							<Button size="small" key={suggestion} onClick={() => add(suggestion)}>
								{suggestion}
							</Button>
						))}
					</div>
				) : null}
			</div>
		</FieldGroup>
	);
}
