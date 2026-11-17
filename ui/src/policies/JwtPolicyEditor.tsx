import { DatabaseZap, FileKey2, Globe2, ShieldCheck } from 'lucide-react';
import { useState } from 'react';

import { LostFieldsWarning, UnsupportedInForm, useMergedEdits } from '@/components/EditorContracts';
import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { Field, FieldGroup } from '@/components/Primitives';
import type { JWTValidationOptions, LocalJwtConfig } from '@/gateway-config';
import {
	authorizationLocationFrom,
	authorizationLocationToValue,
	CredentialLocationSetting
} from '@/policies/AuthorizationLocation';
import { ListEditor } from '@/policies/ListEditor';
import { PolicySection } from '@/policies/PolicyLayout';
import { cleanEmpty, toText } from '@/policies/policyUtils';
import type { JwtPolicy } from '@/policies/types';
import type { SchemaHelp } from '@/schemaHelp';

type JwtMode = 'strict' | 'optional' | 'permissive';
type JwksMode = 'file' | 'url' | 'inline';

type JwtDraft = Omit<JwtPolicy, 'location' | 'jwtValidationOptions'> & {
	location?: unknown;
	jwtValidationOptions?: {
		requiredClaims?: string[];
	};
};

const commonClaims = ['exp', 'nbf', 'aud', 'iss', 'sub'] as const;

const modeOptions: Array<{
	value: JwtMode;
	label: string;
	description: string;
}> = [
	{
		value: 'strict',
		label: 'Strict',
		description: 'Reject requests that do not carry a valid token.'
	},
	{
		value: 'optional',
		label: 'Optional',
		description: 'Validate a token when one is present.'
	},
	{
		value: 'permissive',
		label: 'Permissive',
		description: 'Keep serving traffic while surfacing JWT data when possible.'
	}
];

const jwksOptions: Array<{
	value: JwksMode;
	label: string;
	description: string;
}> = [
	{
		value: 'url',
		label: 'Remote URL',
		description: 'Fetch signing keys from the issuer JWKS endpoint.'
	},
	{
		value: 'file',
		label: 'Local file',
		description: 'Read signing keys from a file on the gateway host.'
	},
	{
		value: 'inline',
		label: 'Inline JSON',
		description: 'Paste a JWKS document directly into the policy.'
	}
];

export function JwtPolicyEditor(props: {
	formId?: string;
	jwt: JwtDraft | null | undefined;
	help: SchemaHelp;
	saving: boolean;
	onSave: (jwt: JwtPolicy) => void;
}) {
	const initialJwks = props.jwt?.jwks;
	const initialJwksMode: JwksMode =
		isRecord(initialJwks) && typeof initialJwks.url === 'string'
			? 'url'
			: isRecord(initialJwks) && typeof initialJwks.file === 'string'
				? 'file'
				: initialJwks
					? 'inline'
					: 'url';

	const [mode, setMode] = useState<JwtMode>(props.jwt?.mode ?? 'optional');
	const [location, setLocation] = useState(() => authorizationLocationFrom(props.jwt?.location));
	const [issuer, setIssuer] = useState(props.jwt?.issuer ?? '');
	const [audiences, setAudiences] = useState(props.jwt?.audiences ?? []);
	const [jwksMode, setJwksMode] = useState<JwksMode>(initialJwksMode);
	const [jwksFile, setJwksFile] = useState(
		isRecord(initialJwks) && typeof initialJwks.file === 'string' ? initialJwks.file : ''
	);
	const [jwksUrl, setJwksUrl] = useState(
		isRecord(initialJwks) && typeof initialJwks.url === 'string' ? initialJwks.url : ''
	);
	const [jwksInline, setJwksInline] = useState(
		initialJwksMode === 'inline' ? toText(initialJwks ?? { keys: [] }) : '{\n  "keys": []\n}'
	);
	const [requiredClaims, setRequiredClaims] = useState(
		() => new Set(props.jwt?.jwtValidationOptions?.requiredClaims ?? ['exp'])
	);
	const [inlineError, setInlineError] = useState<string | null>(null);

	const preview = buildJwtPolicy();
	const merged = useMergedEdits(props.jwt, preview);

	if (props.jwt && 'providers' in props.jwt) {
		return <UnsupportedInForm supports="a single issuer" />;
	}

	function buildJwtPolicy() {
		return {
			...(cleanEmpty({
				mode,
				location: authorizationLocationToValue(location),
				issuer,
				audiences,
				jwks: buildJwks()
			}) as JwtPolicy),
			jwtValidationOptions: {
				requiredClaims: Array.from(requiredClaims)
			}
		} as JwtPolicy;
	}

	function buildJwks() {
		if (jwksMode === 'file') return jwksFile.trim() ? { file: jwksFile.trim() } : undefined;
		if (jwksMode === 'url') return jwksUrl.trim() ? { url: jwksUrl.trim() } : undefined;
		return jwksInline.trim() ? jwksInline : undefined;
	}

	function save() {
		if (jwksMode === 'inline') {
			try {
				JSON.parse(jwksInline);
			} catch {
				setInlineError('Inline JWKS must be valid JSON.');
				return;
			}
		}
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
				icon={<ShieldCheck size={17} />}
				title="Enforcement"
				description="Choose how the gateway behaves when a request has no token or a token cannot be verified."
			>
				<FieldGroup
					label="Validation mode"
					tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'mode')}
				>
					<div className="option-card-grid">
						{modeOptions.map(option => (
							<button
								className={mode === option.value ? 'option-card active' : 'option-card'}
								type="button"
								key={option.value}
								onClick={() => setMode(option.value)}
							>
								<strong>{option.label}</strong>
								<span>{option.description}</span>
							</button>
						))}
					</div>
				</FieldGroup>
			</PolicySection>

			<PolicySection
				icon={jwksMode === 'url' ? <DatabaseZap size={17} /> : <FileKey2 size={17} />}
				title="Signing keys"
				description="Configure the JWKS source used to verify token signatures."
			>
				<FieldGroup
					label="JWKS source"
					tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'jwks')}
				>
					<div className="option-card-grid">
						{jwksOptions.map(option => (
							<button
								className={jwksMode === option.value ? 'option-card active' : 'option-card'}
								type="button"
								key={option.value}
								onClick={() => setJwksMode(option.value)}
							>
								<strong>{option.label}</strong>
								<span>{option.description}</span>
							</button>
						))}
					</div>
				</FieldGroup>
				{jwksMode === 'file' ? (
					<Field
						label="JWKS file"
						tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'jwks')}
						path="jwks.file"
					>
						<input
							value={jwksFile}
							onChange={event => setJwksFile(event.target.value)}
							placeholder="./manifests/jwt/pub-key"
						/>
					</Field>
				) : jwksMode === 'url' ? (
					<Field
						label="JWKS URL"
						tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'jwks')}
						path="jwks.url"
					>
						<input
							value={jwksUrl}
							onChange={event => setJwksUrl(event.target.value)}
							placeholder="https://issuer.example.com/.well-known/jwks.json"
						/>
					</Field>
				) : (
					<FieldGroup
						label="Inline JWKS"
						tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'jwks')}
						className={inlineError ? 'invalid' : undefined}
						hint={inlineError ?? undefined}
						path="jwks"
					>
						<MiniMonacoEditor
							language="json"
							value={jwksInline}
							invalid={Boolean(inlineError)}
							onChange={value => {
								setJwksInline(value);
								setInlineError(null);
							}}
						/>
					</FieldGroup>
				)}
			</PolicySection>

			<PolicySection
				icon={<Globe2 size={17} />}
				title="Token validation"
				description="Restrict accepted tokens by issuer, audience, and required claims."
			>
				<Field
					label="Issuer"
					tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'issuer')}
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
					tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'audiences')}
					values={audiences}
					placeholder="api://gateway"
					emptyText="No audience restriction configured."
					onChange={setAudiences}
				/>

				<FieldGroup
					label="Required claims"
					tooltip={props.help.field<JWTValidationOptions>('JWTValidationOptions', 'requiredClaims')}
				>
					<div className="method-grid">
						{commonClaims.map(claim => (
							<button
								className={requiredClaims.has(claim) ? 'choice-pill active' : 'choice-pill'}
								type="button"
								key={claim}
								onClick={() => setRequiredClaims(current => toggleClaim(current, claim))}
							>
								{claim}
							</button>
						))}
					</div>
				</FieldGroup>
			</PolicySection>

			<CredentialLocationSetting
				value={location}
				help={props.help}
				defaultDescription="Default: Authorization: Bearer token"
				description="Override where this policy reads the JWT from."
				onChange={setLocation}
			/>
		</form>
	);
}

function toggleClaim(values: Set<string>, value: string) {
	const next = new Set(values);
	if (next.has(value)) {
		next.delete(value);
	} else {
		next.add(value);
	}
	return next;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}
