import { DatabaseZap, FileKey2, Globe2, KeyRound, ShieldCheck } from 'lucide-react';
import { useState } from 'react';

import { LostFieldsWarning, useMergedEdits } from '@/components/EditorContracts';
import { EnumSelector } from '@/components/EnumSelector';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { Field, FieldGroup, StatusBanner } from '@/components/Primitives';
import type { LocalMcpAuthentication } from '@/gateway-config';
import { ListEditor } from '@/policies/ListEditor';
import { PolicySection } from '@/policies/PolicyLayout';
import { cleanEmpty, parseYamlText, toYamlMappingText } from '@/policies/policyUtils';
import type { McpAuthenticationDraft } from '@/policies/types';
import type { SchemaHelp } from '@/schemaHelp';

type AuthMode = 'strict' | 'optional' | 'permissive';
type JwksMode = 'url' | 'file' | 'inline';

export function McpAuthenticationPolicyEditor(props: {
	formId?: string;
	authentication: McpAuthenticationDraft | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (authentication: McpAuthenticationDraft) => void;
}) {
	const initialJwks = props.authentication?.jwks;
	const initialJwksMode: JwksMode =
		isRecord(initialJwks) && typeof initialJwks.file === 'string'
			? 'file'
			: isRecord(initialJwks) && typeof initialJwks.url === 'string'
				? 'url'
				: 'inline';
	const [mode, setMode] = useState<AuthMode>(props.authentication?.mode ?? 'strict');
	const [issuer, setIssuer] = useState(props.authentication?.issuer ?? '');
	const [audiences, setAudiences] = useState(props.authentication?.audiences ?? []);
	const [clientId, setClientId] = useState(props.authentication?.clientId ?? '');
	const [jwksMode, setJwksMode] = useState<JwksMode>(initialJwksMode);
	const [jwksValue, setJwksValue] = useState(() => jwksText(initialJwks, initialJwksMode));
	const [resourceMetadata, setResourceMetadata] = useState(() =>
		toYamlMappingText(props.authentication?.resourceMetadata)
	);
	const [metadataError, setMetadataError] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	const preview = safeBuild();
	const merged = useMergedEdits(props.authentication, preview);

	function build() {
		return cleanEmpty({
			mode,
			issuer: issuer.trim(),
			audiences,
			clientId: clientId.trim() || null,
			jwks: buildJwks(jwksMode, jwksValue),
			resourceMetadata: resourceMetadata.trim() ? parseYamlText(resourceMetadata) : {}
		}) as McpAuthenticationDraft;
	}

	function safeBuild() {
		try {
			return build();
		} catch {
			// Reported on save.
			return undefined;
		}
	}

	function save() {
		try {
			if (jwksMode === 'inline' && jwksValue.trim()) JSON.parse(jwksValue);
			if (resourceMetadata.trim() && !isRecord(parseYamlText(resourceMetadata))) {
				setMetadataError('Resource metadata must be a YAML mapping.');
				return;
			}
			setMetadataError(null);
			setError(null);
			props.onSave(merged.value as McpAuthenticationDraft);
		} catch (err) {
			setError(err instanceof Error ? err.message : 'Invalid MCP authentication policy');
		}
	}

	return (
		<form
			id={props.formId}
			className="policy-editor-stack"
			onSubmit={event => {
				event.preventDefault();
				save();
			}}
		>
			<LostFieldsWarning lost={merged.lost} />
			<PolicySection
				icon={<ShieldCheck size={17} />}
				title="Enforcement"
				description="Control whether MCP requests must present a valid JWT."
			>
				<FieldGroup
					label="Validation mode"
					tooltip={props.help.field<LocalMcpAuthentication>('LocalMcpAuthentication', 'mode')}
				>
					<EnumSelector
						ariaLabel="Validation mode"
						value={mode}
						schema={props.help.node(['$defs', 'McpAuthenticationMode'])}
						labels={{
							strict: 'Strict',
							optional: 'Optional',
							permissive: 'Permissive'
						}}
						onChange={value => setMode(value as AuthMode)}
					/>
				</FieldGroup>
			</PolicySection>

			<PolicySection
				icon={jwksMode === 'url' ? <DatabaseZap size={17} /> : <FileKey2 size={17} />}
				title="Signing keys"
				description="Configure the JWKS source used to verify token signatures."
			>
				<FieldGroup
					label="JWKS source"
					tooltip={props.help.field<LocalMcpAuthentication>('LocalMcpAuthentication', 'jwks')}
				>
					<div className="segmented-control compact">
						<button
							className={jwksMode === 'url' ? 'active' : ''}
							type="button"
							onClick={() => {
								setJwksMode('url');
								setJwksValue('');
							}}
						>
							Remote URL
						</button>
						<button
							className={jwksMode === 'file' ? 'active' : ''}
							type="button"
							onClick={() => {
								setJwksMode('file');
								setJwksValue('');
							}}
						>
							Local file
						</button>
						<button
							className={jwksMode === 'inline' ? 'active' : ''}
							type="button"
							onClick={() => {
								setJwksMode('inline');
								setJwksValue('{\n  "keys": []\n}');
							}}
						>
							Inline JSON
						</button>
					</div>
				</FieldGroup>
				{jwksMode === 'inline' ? (
					<FieldGroup label="Inline JWKS" path="jwks">
						<MiniMonacoEditor language="json" value={jwksValue} onChange={setJwksValue} />
					</FieldGroup>
				) : (
					<Field
						label={jwksMode === 'url' ? 'JWKS URL' : 'JWKS file'}
						path={jwksMode === 'url' ? 'jwks.url' : 'jwks.file'}
					>
						<input
							value={jwksValue}
							onChange={event => setJwksValue(event.target.value)}
							placeholder={
								jwksMode === 'url'
									? 'https://issuer.example.com/.well-known/jwks.json'
									: '$HOME/.secrets/jwks.json'
							}
						/>
					</Field>
				)}
			</PolicySection>

			<PolicySection
				icon={<Globe2 size={17} />}
				title="Token validation"
				description="Restrict accepted MCP tokens by issuer and audience."
			>
				<Field
					label="Issuer"
					tooltip={props.help.field<LocalMcpAuthentication>('LocalMcpAuthentication', 'issuer')}
					path="issuer"
				>
					<input
						value={issuer}
						onChange={event => setIssuer(event.target.value)}
						placeholder="https://issuer.example.com"
					/>
				</Field>
				<ListEditor
					label="Audiences"
					tooltip={props.help.field<LocalMcpAuthentication>('LocalMcpAuthentication', 'audiences')}
					values={audiences}
					placeholder="mcp://gateway"
					emptyText="No audience restriction configured."
					onChange={setAudiences}
				/>
				<Field
					label="Client ID"
					tooltip={props.help.field<LocalMcpAuthentication>('LocalMcpAuthentication', 'clientId')}
				>
					<input
						value={clientId ?? ''}
						onChange={event => setClientId(event.target.value)}
						placeholder="optional OAuth client ID"
					/>
				</Field>
			</PolicySection>

			<PolicySection
				icon={<KeyRound size={17} />}
				title="Protected resource metadata"
				description="Metadata advertised to MCP clients for OAuth protected resources."
			>
				<FieldGroup
					label="Resource metadata YAML"
					tooltip={props.help.field<LocalMcpAuthentication>(
						'LocalMcpAuthentication',
						'resourceMetadata'
					)}
					className={metadataError ? 'invalid' : undefined}
					hint={metadataError ?? undefined}
					path="resourceMetadata"
				>
					<MiniMonacoEditor
						language="yaml"
						value={resourceMetadata}
						invalid={Boolean(metadataError)}
						onChange={setResourceMetadata}
					/>
				</FieldGroup>
			</PolicySection>

			{error ? (
				<StatusBanner state="bad" title="Invalid MCP authentication policy">
					{error}
				</StatusBanner>
			) : null}
		</form>
	);
}

function buildJwks(mode: JwksMode, value: string) {
	const trimmed = value.trim();
	if (!trimmed) return undefined;
	if (mode === 'url') return { url: trimmed };
	if (mode === 'file') return { file: trimmed };
	JSON.parse(trimmed);
	return trimmed;
}

function jwksText(value: unknown, mode: JwksMode) {
	if (mode === 'url' && isRecord(value) && typeof value.url === 'string') return value.url;
	if (mode === 'file' && isRecord(value) && typeof value.file === 'string') return value.file;
	if (typeof value === 'string') return value;
	return '{\n  "keys": []\n}';
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}
