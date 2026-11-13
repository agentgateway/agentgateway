import { Button, Input } from 'antd';
import { useState } from 'react';

import { LostFieldsWarning, useMergedEdits } from '@/components/EditorContracts';
import { Field, FieldGroup } from '@/components/Primitives';
import { ListEditor } from '@/policies/ListEditor';
import { OptionCheckbox } from '@/policies/PolicyFormControls';
import { appendUnique, cleanEmpty, toggleStringSet } from '@/policies/policyUtils';
import type { SchemaHelp } from '@/schemaHelp';
import type { CorsPolicy } from '@/types';

const corsMethods = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS'] as const;

export function CorsPolicyEditor(props: {
	formId?: string;
	cors: CorsPolicy | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (cors: CorsPolicy) => void;
}) {
	const [origins, setOrigins] = useState(props.cors?.allowOrigins ?? []);
	const [allowCredentials, setAllowCredentials] = useState(Boolean(props.cors?.allowCredentials));
	const [maxAge, setMaxAge] = useState(props.cors?.maxAge ?? '');
	const [allMethods, setAllMethods] = useState(Boolean(props.cors?.allowMethods?.includes('*')));
	const [methods, setMethods] = useState(
		() => new Set((props.cors?.allowMethods ?? ['GET', 'POST']).filter(method => method !== '*'))
	);
	const [allHeaders, setAllHeaders] = useState(
		Boolean(props.cors?.allowHeaders?.includes('*') ?? true)
	);
	const [headers, setHeaders] = useState(
		(props.cors?.allowHeaders ?? []).filter(header => header !== '*')
	);
	const [exposeHeaders, setExposeHeaders] = useState(props.cors?.exposeHeaders ?? []);
	const merged = useMergedEdits(
		props.cors,
		buildCorsPolicy({
			origins,
			allMethods,
			methods,
			allHeaders,
			headers,
			exposeHeaders,
			allowCredentials,
			maxAge
		})
	);

	return (
		<form
			id={props.formId}
			className="policy-editor-stack"
			onSubmit={event => {
				event.preventDefault();
				props.onSave(merged.value);
			}}
		>
			<LostFieldsWarning lost={merged.lost} />
			<ListEditor
				label="Allowed origins"
				tooltip={props.help.field<CorsPolicy>('CorsSerde', 'allowOrigins')}
				values={origins}
				placeholder="http://localhost:19000"
				onChange={setOrigins}
				actions={
					<Button
						onClick={() => setOrigins(current => appendUnique(current, window.location.origin))}
					>
						Add current origin
					</Button>
				}
			/>
			<FieldGroup
				label="Allowed methods"
				tooltip={props.help.field<CorsPolicy>('CorsSerde', 'allowMethods')}
			>
				<div className="method-grid">
					<Button
						shape="round"
						type={allMethods ? 'primary' : 'default'}
						aria-pressed={allMethods}
						onClick={() => setAllMethods(current => !current)}
					>
						ALL
					</Button>
					{corsMethods.map(method => {
						const active = !allMethods && methods.has(method);
						return (
							<Button
								shape="round"
								type={active ? 'primary' : 'default'}
								aria-pressed={active}
								disabled={allMethods}
								key={method}
								onClick={() => setMethods(current => toggleStringSet(current, method))}
							>
								{method}
							</Button>
						);
					})}
				</div>
			</FieldGroup>
			<FieldGroup
				label="Allowed headers"
				tooltip={props.help.field<CorsPolicy>('CorsSerde', 'allowHeaders')}
			>
				<OptionCheckbox
					checked={allHeaders}
					onChange={setAllHeaders}
					title="Allow all request headers"
					description="Accept any request header in browser preflight checks"
				/>
			</FieldGroup>
			{!allHeaders ? (
				<ListEditor
					label="Header allowlist"
					values={headers}
					placeholder="authorization"
					suggestions={['authorization', 'content-type', 'mcp-session-id']}
					onChange={setHeaders}
				/>
			) : null}
			<ListEditor
				label="Expose headers"
				tooltip={props.help.field<CorsPolicy>('CorsSerde', 'exposeHeaders')}
				values={exposeHeaders}
				placeholder="mcp-session-id"
				suggestions={['mcp-session-id', 'x-request-id']}
				onChange={setExposeHeaders}
			/>
			<div className="form-grid">
				<FieldGroup
					label="Credentials"
					tooltip={props.help.field<CorsPolicy>('CorsSerde', 'allowCredentials')}
				>
					<OptionCheckbox
						checked={allowCredentials}
						onChange={setAllowCredentials}
						title="Allow credentials"
						description="Permit browser credentials on CORS requests"
					/>
				</FieldGroup>
				<Field
					label="Max age"
					path="maxAge"
					tooltip={props.help.field<CorsPolicy>('CorsSerde', 'maxAge')}
				>
					<Input
						value={maxAge}
						onChange={event => setMaxAge(event.target.value)}
						placeholder="24h"
					/>
				</Field>
			</div>
		</form>
	);
}

function buildCorsPolicy(args: {
	origins: string[];
	allMethods: boolean;
	methods: Set<string>;
	allHeaders: boolean;
	headers: string[];
	exposeHeaders: string[];
	allowCredentials: boolean;
	maxAge: string;
}): CorsPolicy {
	return cleanEmpty({
		allowOrigins: args.origins,
		allowHeaders: args.allHeaders ? ['*'] : args.headers,
		allowMethods: args.allMethods ? ['*'] : Array.from(args.methods),
		exposeHeaders: args.exposeHeaders,
		allowCredentials: args.allowCredentials ? true : undefined,
		maxAge: args.maxAge.trim() || undefined
	}) as CorsPolicy;
}
