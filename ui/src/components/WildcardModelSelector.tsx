import { CatalogModelSelector } from '@/components/CatalogModelSelector';
import {
	wildcardCatalogPrefix,
	wildcardMatchesModel,
	wildcardModelPrefix,
	wildcardResolvedSuffix
} from '@/modelResolution';
import type { LlmModel } from '@/types';

/** Picks the concrete name a client requests from a wildcard model. */
export function WildcardModelSelector(props: {
	id?: string;
	ariaLabel: string;
	'aria-describedby'?: string;
	model: LlmModel;
	value: string;
	onChange: (value: string) => void;
	provider?: string | null;
	placeholder?: string;
}) {
	const prefix = wildcardModelPrefix(props.model.name);
	const catalogPrefix = wildcardCatalogPrefix(props.model);
	const value = wildcardResolvedSuffix(props.value, props.model.name, prefix);
	const onChange = (suffix: string) => props.onChange(`${prefix}${suffix}`);
	return (
		<div className="target-resolved-composite">
			{prefix ? <span className="target-prefix">{prefix}</span> : null}
			{catalogPrefix === undefined ? (
				<input
					id={props.id}
					aria-label={props.ariaLabel}
					aria-describedby={props['aria-describedby']}
					value={value}
					onChange={event => onChange(event.target.value)}
					placeholder="Model name"
				/>
			) : (
				<CatalogModelSelector
					id={props.id}
					ariaLabel={props.ariaLabel}
					aria-describedby={props['aria-describedby']}
					value={value}
					provider={props.provider}
					mapOption={id => {
						const name = `${catalogPrefix}${id}`;
						return wildcardMatchesModel(props.model.name, name)
							? name.slice(prefix.length)
							: undefined;
					}}
					onChange={onChange}
					placeholder={props.placeholder}
				/>
			)}
		</div>
	);
}
