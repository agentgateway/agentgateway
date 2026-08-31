import { DatabaseZap, FileKey2, Globe2, ShieldCheck } from 'lucide-react';
import { useState } from 'react';

import { MiniMonacoEditor } from '@/components/MiniMonacoEditor';
import { Field, FieldGroup, StatusBanner } from '@/components/Primitives';
import type { JWTValidationOptions, LocalJwtConfig } from '@/gateway-config';
import { tr } from '@/i18n';
import {
	authorizationLocationFrom,
	authorizationLocationToValue,
	CredentialLocationSetting
} from '@/policies/AuthorizationLocation';
import { ListEditor } from '@/policies/ListEditor';
import { PolicySection } from '@/policies/PolicyLayout';
import { cleanEmpty, toText } from '@/policies/policyUtils';
import { ResultingYaml } from '@/policies/ResultingYaml';
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

type JwtFieldErrors = Partial<Record<'issuer' | 'jwksUrl' | 'jwksFile' | 'jwksInline', string>>;

const commonClaims = ['exp', 'nbf', 'aud', 'iss', 'sub'] as const;

const modeOptions: Array<{
	value: JwtMode;
	label: string;
	description: string;
}> = [
	{
		value: 'strict',
		get label() {
			return tr('copy.strict');
		},
		get description() {
			return tr('copy.rejectRequestsThatDoNotCarryAValidToken');
		}
	},
	{
		value: 'optional',
		get label() {
			return tr('copy.optional_1yfbac9');
		},
		get description() {
			return tr('copy.validateATokenWhenOneIsPresent');
		}
	},
	{
		value: 'permissive',
		get label() {
			return tr('copy.permissive');
		},
		get description() {
			return tr('copy.keepServingTrafficWhileSurfacingJwtDataWhenPossible');
		}
	}
];

const jwksOptions: Array<{
	value: JwksMode;
	label: string;
	description: string;
}> = [
	{
		value: 'url',
		get label() {
			return tr('copy.remoteUrl');
		},
		get description() {
			return tr('copy.fetchSigningKeysFromTheIssuerJwksEndpoint');
		}
	},
	{
		value: 'file',
		get label() {
			return tr('copy.localFile');
		},
		get description() {
			return tr('copy.readSigningKeysFromAFileOnTheGatewayHost');
		}
	},
	{
		value: 'inline',
		get label() {
			return tr('copy.inlineJson');
		},
		get description() {
			return tr('copy.pasteAJwksDocumentDirectlyIntoThePolicy');
		}
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

	const [mode, setMode] = useState<JwtMode>(props.jwt?.mode ?? 'strict');
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
	const [fieldErrors, setFieldErrors] = useState<JwtFieldErrors>({});
	const [error, setError] = useState<string | null>(null);

	const preview = safeBuildJwtPolicy();

	function buildJwtPolicy() {
		return cleanEmpty({
			mode,
			location: authorizationLocationToValue(location),
			issuer,
			audiences,
			jwks: buildJwks(),
			jwtValidationOptions: {
				requiredClaims: Array.from(requiredClaims)
			}
		}) as JwtPolicy;
	}

	function buildJwks() {
		if (jwksMode === 'file') return jwksFile.trim() ? { file: jwksFile.trim() } : undefined;
		if (jwksMode === 'url') return jwksUrl.trim() ? { url: jwksUrl.trim() } : undefined;
		if (!jwksInline.trim()) return undefined;
		JSON.parse(jwksInline);
		return jwksInline;
	}

	function safeBuildJwtPolicy() {
		try {
			return buildJwtPolicy();
		} catch {
			return {
				error: 'Inline JWKS must be valid JSON before it can be saved.'
			};
		}
	}

	function save() {
		try {
			setError(null);
			const validationErrors = validateJwtPolicy();
			setFieldErrors(validationErrors);
			if (Object.keys(validationErrors).length) {
				setError(tr('copy.fixTheHighlightedFieldsBeforeSaving'));
				return;
			}
			props.onSave(buildJwtPolicy());
		} catch (err) {
			setError(err instanceof Error ? err.message : 'Invalid JWT policy');
		}
	}

	function validateJwtPolicy() {
		const errors: JwtFieldErrors = {};
		if (!issuer.trim()) errors.issuer = 'Issuer is required.';
		if (jwksMode === 'url' && !jwksUrl.trim()) errors.jwksUrl = 'JWKS URL is required.';
		if (jwksMode === 'file' && !jwksFile.trim()) errors.jwksFile = 'JWKS file is required.';
		if (jwksMode === 'inline') {
			if (!jwksInline.trim()) {
				errors.jwksInline = 'Inline JWKS is required.';
			} else {
				try {
					JSON.parse(jwksInline);
				} catch {
					errors.jwksInline = 'Inline JWKS must be valid JSON.';
				}
			}
		}
		return errors;
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
			<PolicySection
				icon={<ShieldCheck size={17} />}
				title={tr('copy.enforcement')}
				description={tr(
					'copy.chooseHowTheGatewayBehavesWhenARequestHasNoTokenOrATokenCannotBeVerified'
				)}
			>
				<FieldGroup
					label={tr('copy.validationMode')}
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
				title={tr('copy.signingKeys')}
				description={tr('copy.configureTheJwksSourceUsedToVerifyTokenSignatures')}
			>
				<FieldGroup
					label={tr('copy.jwksSource')}
					tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'jwks')}
				>
					<div className="option-card-grid">
						{jwksOptions.map(option => (
							<button
								className={jwksMode === option.value ? 'option-card active' : 'option-card'}
								type="button"
								key={option.value}
								onClick={() => {
									setJwksMode(option.value);
									clearJwksErrors();
								}}
							>
								<strong>{option.label}</strong>
								<span>{option.description}</span>
							</button>
						))}
					</div>
				</FieldGroup>
				{jwksMode === 'file' ? (
					<Field
						label={tr('copy.jwksFile')}
						tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'jwks')}
						className={fieldErrors.jwksFile ? 'invalid' : undefined}
						hint={fieldErrors.jwksFile}
					>
						<input
							value={jwksFile}
							aria-invalid={Boolean(fieldErrors.jwksFile)}
							onChange={event => {
								setJwksFile(event.target.value);
								clearFieldError('jwksFile');
							}}
							placeholder="./manifests/jwt/pub-key"
						/>
					</Field>
				) : jwksMode === 'url' ? (
					<Field
						label={tr('copy.jwksUrl')}
						tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'jwks')}
						className={fieldErrors.jwksUrl ? 'invalid' : undefined}
						hint={fieldErrors.jwksUrl}
					>
						<input
							value={jwksUrl}
							aria-invalid={Boolean(fieldErrors.jwksUrl)}
							onChange={event => {
								setJwksUrl(event.target.value);
								clearFieldError('jwksUrl');
							}}
							placeholder="https://issuer.example.com/.well-known/jwks.json"
						/>
					</Field>
				) : (
					<FieldGroup
						label={tr('copy.inlineJwks')}
						tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'jwks')}
						className={fieldErrors.jwksInline ? 'invalid' : undefined}
						hint={fieldErrors.jwksInline}
					>
						<MiniMonacoEditor
							language="json"
							value={jwksInline}
							invalid={Boolean(fieldErrors.jwksInline)}
							onChange={value => {
								setJwksInline(value);
								clearFieldError('jwksInline');
							}}
						/>
					</FieldGroup>
				)}
			</PolicySection>

			<PolicySection
				icon={<Globe2 size={17} />}
				title={tr('copy.tokenValidation')}
				description={tr('copy.restrictAcceptedTokensByIssuerAudienceAndRequiredClaims')}
			>
				<Field
					label={tr('copy.issuer')}
					tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'issuer')}
					className={fieldErrors.issuer ? 'invalid' : undefined}
					hint={fieldErrors.issuer}
				>
					<input
						value={issuer}
						aria-invalid={Boolean(fieldErrors.issuer)}
						onChange={event => {
							setIssuer(event.target.value);
							clearFieldError('issuer');
						}}
						placeholder="https://issuer.example.com"
					/>
				</Field>

				<ListEditor
					label={tr('copy.audiences')}
					tooltip={props.help.field<LocalJwtConfig>('LocalJwtConfig', 'audiences')}
					values={audiences}
					placeholder="api://gateway"
					emptyText={tr('copy.noAudienceRestrictionConfigured')}
					onChange={setAudiences}
				/>

				<FieldGroup
					label={tr('copy.requiredClaims')}
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
				description={tr('copy.overrideWhereThisPolicyReadsTheCredential')}
				onChange={setLocation}
			/>

			<ResultingYaml value={preview} />

			{error ? (
				<StatusBanner state="bad" title={tr('copy.invalidJwtPolicy')}>
					{error}
				</StatusBanner>
			) : null}
		</form>
	);

	function clearFieldError(field: keyof JwtFieldErrors) {
		setFieldErrors(current => {
			if (!current[field]) return current;
			const next = { ...current };
			delete next[field];
			return next;
		});
		setError(null);
	}

	function clearJwksErrors() {
		setFieldErrors(current => {
			const next = { ...current };
			delete next.jwksUrl;
			delete next.jwksFile;
			delete next.jwksInline;
			return next;
		});
		setError(null);
	}
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
