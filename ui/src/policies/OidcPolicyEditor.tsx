import { Fingerprint, Globe2, KeyRound } from 'lucide-react';
import { useState } from 'react';

import { LostFieldsWarning, useMergedEdits } from '@/components/EditorContracts';
import { EnumSelector } from '@/components/EnumSelector';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { Field, FieldGroup } from '@/components/Primitives';
import type { LocalOidcConfig } from '@/gateway-config';
import { ListEditor } from '@/policies/ListEditor';
import { PolicySection } from '@/policies/PolicyLayout';
import { cleanEmpty, isRecord } from '@/policies/policyUtils';
import type { OidcDraft } from '@/policies/types';
import type { SchemaHelp } from '@/schemaHelp';

type ProviderMode = 'discovery' | 'explicit';
type SourceMode = 'none' | 'url' | 'file' | 'inline';
type TokenEndpointAuth = 'clientSecretBasic' | 'clientSecretPost';

type SourceDraft = {
	mode: SourceMode;
	value: string;
};

const providerModes: Array<{
	value: ProviderMode;
	label: string;
	description: string;
}> = [
	{
		value: 'discovery',
		label: 'Discovery',
		description: 'Use the issuer metadata endpoint unless an override is provided.'
	},
	{
		value: 'explicit',
		label: 'Explicit endpoints',
		description: 'Manually provide authorization, token, and signing-key metadata.'
	}
];

const sourceModes: Array<{ value: SourceMode; label: string }> = [
	{ value: 'none', label: 'None' },
	{ value: 'url', label: 'Remote URL' },
	{ value: 'file', label: 'Local file' },
	{ value: 'inline', label: 'Inline JSON' }
];

const tokenEndpointAuthOptions: Array<{
	value: TokenEndpointAuth;
	label: string;
	searchText: string;
}> = [
	{
		value: 'clientSecretBasic',
		label: 'Client secret basic',
		searchText: 'clientSecretBasic basic'
	},
	{
		value: 'clientSecretPost',
		label: 'Client secret post',
		searchText: 'clientSecretPost post'
	}
];

const issuerSuggestions = [
	{ label: 'Google', value: 'https://accounts.google.com' },
	{
		label: 'Microsoft Entra',
		value: 'https://login.microsoftonline.com/{tenant}/v2.0'
	},
	{ label: 'Okta', value: 'https://{yourOktaDomain}/oauth2/default' },
	{ label: 'Auth0', value: 'https://{yourDomain}.auth0.com/' },
	{ label: 'Keycloak', value: 'http://localhost:7080/realms/{realm}' }
];

export function OidcPolicyEditor(props: {
	formId?: string;
	oidc: OidcDraft | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (oidc: OidcDraft) => void;
}) {
	const hasExplicitProvider = Boolean(
		props.oidc?.authorizationEndpoint || props.oidc?.tokenEndpoint || props.oidc?.jwks
	);
	const [providerMode, setProviderMode] = useState<ProviderMode>(
		hasExplicitProvider ? 'explicit' : 'discovery'
	);
	const [issuer, setIssuer] = useState(props.oidc?.issuer ?? '');
	const [clientId, setClientId] = useState(props.oidc?.clientId ?? '');
	const [clientSecret, setClientSecret] = useState(props.oidc?.clientSecret ?? '');
	const [redirectURI, setRedirectURI] = useState(props.oidc?.redirectURI ?? '');
	const [scopes, setScopes] = useState(props.oidc?.scopes ?? []);
	const [discovery, setDiscovery] = useState<SourceDraft>(() =>
		sourceFrom(props.oidc?.discovery, 'none')
	);
	const [authorizationEndpoint, setAuthorizationEndpoint] = useState(
		props.oidc?.authorizationEndpoint ?? ''
	);
	const [tokenEndpoint, setTokenEndpoint] = useState(props.oidc?.tokenEndpoint ?? '');
	const [tokenEndpointAuth, setTokenEndpointAuth] = useState<TokenEndpointAuth>(
		props.oidc?.tokenEndpointAuth ?? 'clientSecretBasic'
	);
	const [jwks, setJwks] = useState<SourceDraft>(() => sourceFrom(props.oidc?.jwks, 'url'));

	const preview = buildOidcPolicy();
	const merged = useMergedEdits(props.oidc, preview);

	function buildOidcPolicy() {
		return cleanEmpty({
			issuer,
			discovery: providerMode === 'discovery' ? sourceToConfig(discovery) : undefined,
			authorizationEndpoint: providerMode === 'explicit' ? authorizationEndpoint : undefined,
			tokenEndpoint: providerMode === 'explicit' ? tokenEndpoint : undefined,
			tokenEndpointAuth: providerMode === 'explicit' ? tokenEndpointAuth : undefined,
			jwks: providerMode === 'explicit' ? sourceToConfig(jwks) : undefined,
			clientId,
			clientSecret,
			redirectURI,
			scopes
		}) as OidcDraft;
	}

	function save() {
		props.onSave(merged.value);
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
				icon={<Fingerprint size={17} />}
				title="Provider"
				description="Configure where browser login starts and how returned ID tokens are validated."
			>
				<Field
					label="Issuer"
					tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'issuer')}
					path="issuer"
				>
					{(id, hintId, invalid) => (
						<>
							<input
								id={id}
								aria-describedby={hintId}
								aria-invalid={invalid}
								value={issuer}
								onChange={event => setIssuer(event.target.value)}
								placeholder="https://issuer.example.com"
							/>
							<div className="suggestion-row">
								{issuerSuggestions.map(suggestion => (
									<button
										className="table-action"
										type="button"
										key={suggestion.label}
										onClick={() => setIssuer(suggestion.value)}
									>
										{suggestion.label}
									</button>
								))}
							</div>
						</>
					)}
				</Field>

				<FieldGroup
					label="Provider metadata"
					tooltip={props.help.field<LocalOidcConfig>(
						'LocalOidcConfig',
						'authorizationEndpoint',
						'OIDC provider endpoints can be discovered from the issuer or supplied explicitly.'
					)}
				>
					<div className="option-card-grid">
						{providerModes.map(mode => (
							<button
								className={providerMode === mode.value ? 'option-card active' : 'option-card'}
								type="button"
								key={mode.value}
								onClick={() => setProviderMode(mode.value)}
							>
								<strong>{mode.label}</strong>
								<span>{mode.description}</span>
							</button>
						))}
					</div>
				</FieldGroup>

				{providerMode === 'discovery' ? (
					<SourceEditor
						label="Discovery override"
						tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'discovery')}
						value={discovery}
						missing={discovery.mode === 'none' ? undefined : !discovery.value.trim()}
						optionalText="Default: issuer + /.well-known/openid-configuration"
						onChange={setDiscovery}
					/>
				) : (
					<>
						<Field
							label="Authorization endpoint"
							tooltip={props.help.field<LocalOidcConfig>(
								'LocalOidcConfig',
								'authorizationEndpoint'
							)}
							missing={!authorizationEndpoint.trim()}
						>
							<input
								value={authorizationEndpoint}
								onChange={event => setAuthorizationEndpoint(event.target.value)}
								placeholder="https://issuer.example.com/oauth2/v1/authorize"
							/>
						</Field>
						<Field
							label="Token endpoint"
							tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'tokenEndpoint')}
							missing={!tokenEndpoint.trim()}
						>
							<input
								value={tokenEndpoint}
								onChange={event => setTokenEndpoint(event.target.value)}
								placeholder="https://issuer.example.com/oauth2/v1/token"
							/>
						</Field>
						<FieldGroup
							label="Token endpoint auth"
							tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'tokenEndpointAuth')}
						>
							<EnumSelector
								value={tokenEndpointAuth}
								ariaLabel="Token endpoint auth"
								options={tokenEndpointAuthOptions}
								schema={props.help.fieldNode<LocalOidcConfig>(
									'LocalOidcConfig',
									'tokenEndpointAuth'
								)}
								onChange={setTokenEndpointAuth}
							/>
						</FieldGroup>
						<SourceEditor
							label="JWKS"
							tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'jwks')}
							value={jwks}
							missing={!sourceToConfig(jwks)}
							onChange={setJwks}
						/>
					</>
				)}
			</PolicySection>

			<PolicySection
				icon={<KeyRound size={17} />}
				title="Client"
				description="Identify the OAuth2 client used by the gateway during the authorization code flow."
			>
				<Field
					label="Client ID"
					tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'clientId')}
					path="clientId"
				>
					<input
						value={clientId}
						onChange={event => setClientId(event.target.value)}
						placeholder="agentgateway-browser"
					/>
				</Field>
				<Field
					label="Client secret"
					tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'clientSecret')}
					path="clientSecret"
				>
					<input
						type="text"
						className="masked-secret-input"
						autoComplete="off"
						autoCorrect="off"
						autoCapitalize="none"
						data-1p-ignore="true"
						data-lpignore="true"
						data-form-type="other"
						name="agw-oidc-client-secret"
						spellCheck={false}
						value={clientSecret}
						onChange={event => setClientSecret(event.target.value)}
						placeholder="OAuth2 client secret"
					/>
				</Field>
				<Field
					label="Redirect URI"
					tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'redirectURI')}
					path="redirectURI"
				>
					<input
						value={redirectURI}
						onChange={event => setRedirectURI(event.target.value)}
						placeholder="http://localhost:4000/oauth/callback"
					/>
				</Field>
			</PolicySection>

			<PolicySection
				icon={<Globe2 size={17} />}
				title="Scopes"
				description="Request extra OAuth2 scopes. The gateway always includes openid."
			>
				<ListEditor
					label="Additional scopes"
					tooltip={props.help.field<LocalOidcConfig>('LocalOidcConfig', 'scopes')}
					values={scopes}
					placeholder="profile"
					emptyText="No additional scopes configured."
					suggestions={['profile', 'email', 'offline_access']}
					onChange={setScopes}
				/>
			</PolicySection>
		</form>
	);
}

function SourceEditor(props: {
	label: string;
	value: SourceDraft;
	onChange: (value: SourceDraft) => void;
	// As for Field.
	missing?: boolean;
	tooltip?: string;
	optionalText?: string;
}) {
	const activeMode = props.value.mode;
	const placeholder =
		activeMode === 'file'
			? './manifests/oidc/provider.json'
			: activeMode === 'url'
				? 'https://issuer.example.com/.well-known/openid-configuration'
				: '{\n  "keys": []\n}';

	return (
		<FieldGroup label={props.label} tooltip={props.tooltip} missing={props.missing}>
			<div className="oidc-source">
				<EnumSelector
					value={activeMode}
					ariaLabel={props.label}
					options={sourceModes}
					onChange={mode =>
						props.onChange({
							mode,
							value: mode === 'none' ? '' : props.value.value
						})
					}
				/>
				{activeMode === 'none' ? (
					<div className="empty-inline">{props.optionalText ?? 'No source configured.'}</div>
				) : activeMode === 'inline' ? (
					<MiniMonacoEditor
						language="json"
						value={props.value.value}
						onChange={value => props.onChange({ ...props.value, value })}
						placeholder={placeholder}
					/>
				) : (
					<input
						value={props.value.value}
						onChange={event => props.onChange({ ...props.value, value: event.target.value })}
						placeholder={placeholder}
					/>
				)}
			</div>
		</FieldGroup>
	);
}

function sourceFrom(value: unknown, emptyMode: SourceMode): SourceDraft {
	if (isRecord(value) && typeof value.url === 'string') return { mode: 'url', value: value.url };
	if (isRecord(value) && typeof value.file === 'string') return { mode: 'file', value: value.file };
	if (typeof value === 'string') return { mode: 'inline', value };
	return { mode: emptyMode, value: '' };
}

function sourceToConfig(source: SourceDraft) {
	const value = source.value.trim();
	if (source.mode === 'none' || !value) return undefined;
	if (source.mode === 'url') return { url: value };
	if (source.mode === 'file') return { file: value };
	return value;
}
