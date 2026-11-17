import { Trash2 } from 'lucide-react';
import { useRef, useState } from 'react';

import { ConfigDiffSaveActions, ConfigSaveButton } from '@/components/ConfigDiffDrawer';
import { EditorSchemaContext, EditorValueContext } from '@/components/EditorContracts';
import { ConfirmDialog, Drawer, type DrawerRaw, Tooltip } from '@/components/Primitives';
import type { BackendAuth, LoggingPolicy, TracingConfig } from '@/gateway-config';
import { takeHybridFileWriteOverride } from '@/hooks';
import { AccessLogPolicyEditor } from '@/policies/AccessLogPolicyEditor';
import { AuthorizationPolicyEditor } from '@/policies/AuthorizationPolicyEditor';
import { BackendAuthPolicyEditor } from '@/policies/backendAuth';
import { CorsPolicyEditor } from '@/policies/CorsPolicyEditor';
import { ExtAuthzPolicyEditor } from '@/policies/ExtAuthzPolicyEditor';
import { ExtProcPolicyEditor } from '@/policies/ExtProcPolicyEditor';
import { GenericPolicyEditor } from '@/policies/GenericPolicyEditor';
import { JwtPolicyEditor } from '@/policies/JwtPolicyEditor';
import { LocalRateLimitPolicyEditor } from '@/policies/LocalRateLimitPolicyEditor';
import { McpAuthenticationPolicyEditor } from '@/policies/McpAuthenticationPolicyEditor';
import { McpGuardrailsPolicyEditor } from '@/policies/McpGuardrailsPolicyEditor';
import { OidcPolicyEditor } from '@/policies/OidcPolicyEditor';
import { policyEnabled } from '@/policies/policyUtils';
import { RemoteRateLimitPolicyEditor } from '@/policies/RemoteRateLimitPolicyEditor';
import { TracingPolicyEditor } from '@/policies/TracingPolicyEditor';
import { TransformationsPolicyEditor } from '@/policies/TransformationsPolicyEditor';
import type {
	AuthorizationDraft,
	ExtAuthzDraft,
	ExtProcDraft,
	JwtPolicy,
	LocalRateLimitConfig,
	McpAuthenticationDraft,
	McpGuardrailsDraft,
	OidcDraft,
	RemoteRateLimitDraft,
	TransformationDraft
} from '@/policies/types';
import type { SchemaHelp } from '@/schemaHelp';
import type { CorsPolicy, GatewayConfig } from '@/types';

export type PolicyEditorKind =
	| 'accessLog'
	| 'authorization'
	| 'backendAuth'
	| 'cors'
	| 'extAuthz'
	| 'extProc'
	| 'jwtAuth'
	| 'localRateLimit'
	| 'mcpAuthentication'
	| 'mcpAuthorization'
	| 'mcpGuardrails'
	| 'oidc'
	| 'remoteRateLimit'
	| 'tracing'
	| 'transformations';

export function PolicyDrawer(props: {
	policyKey: string;
	title: string;
	customEditor?: PolicyEditorKind;
	policyValue: unknown;
	policies?: Record<string, unknown> | null;
	help: SchemaHelp;
	saving: boolean;
	saveError?: string | null;
	schemaRoot?: string;
	config?: GatewayConfig | null;
	databaseBacked?: boolean;
	onClose: () => void;
	applySaveDiff?: (config: GatewayConfig, value: unknown) => void;
	onSave: (value: unknown) => void;
	onDisable: () => void;
}) {
	const enabled = policyEnabled(props.policies, props.policyKey);
	const [confirmDelete, setConfirmDelete] = useState(false);
	const submittedValue = useRef<unknown>(undefined);
	const formId = `policy-editor-${sanitizePolicyFormId(props.schemaRoot ?? 'LocalLLMPolicy')}-${sanitizePolicyFormId(props.policyKey)}`;
	const saveResourceDiff = props.databaseBacked
		? () => ({
				original: props.policyValue ?? {},
				modified: submittedValue.current ?? {}
			})
		: undefined;

	const policy = usePolicyRaw({
		formId,
		policyValue: props.policyValue,
		schemaPath: ['$defs', props.schemaRoot ?? 'LocalLLMPolicy', 'properties', props.policyKey],
		onSave: value => {
			submittedValue.current = value;
		}
	});

	function submitPolicyForm() {
		submittedValue.current = undefined;
		policy.submit();
		return submittedValue.current !== undefined;
	}

	return (
		<>
			<Drawer
				title={props.title}
				onClose={props.onClose}
				raw={props.customEditor ? policy.raw : undefined}
				error={props.saveError}
				headerActions={
					enabled ? (
						<ConfigSaveButton
							disabled={props.saving}
							allowHybridWrite={props.databaseBacked}
							onClick={() => setConfirmDelete(true)}
						>
							<Trash2 size={16} />
							Delete policy
						</ConfigSaveButton>
					) : (
						<Tooltip content="Policy is not enabled">
							<button
								className="icon-button danger"
								type="button"
								aria-label="Delete policy"
								disabled
							>
								<Trash2 size={17} />
							</button>
						</Tooltip>
					)
				}
				footer={(_requestClose, dirty) => (
					<ConfigDiffSaveActions
						config={props.config}
						diffTitle={`${props.title} policy config diff`}
						saveLabel="Save policy"
						saving={props.saving}
						diffDisabled={enabled && !dirty}
						resourceDiff={saveResourceDiff}
						onSave={() => {
							if (!submitPolicyForm()) return;
							// An emptied raw policy is no policy.
							if (submittedValue.current === null) props.onDisable();
							else props.onSave(submittedValue.current);
						}}
						beforeDiff={submitPolicyForm}
						applyDiff={next => {
							if (props.applySaveDiff) {
								props.applySaveDiff(next, submittedValue.current);
							}
						}}
					/>
				)}
			>
				<EditorValueContext.Provider value={policy.report}>
					<PolicyEditorBody
						key={JSON.stringify(policy.value ?? null)}
						formId={formId}
						policyKey={props.policyKey}
						customEditor={props.customEditor}
						policyValue={policy.value}
						help={props.help}
						saving={props.saving}
						schemaRoot={props.schemaRoot}
						onSave={value => {
							submittedValue.current = value;
						}}
					/>
				</EditorValueContext.Provider>
			</Drawer>
			{confirmDelete ? (
				<ConfirmDialog
					title="Delete policy?"
					destructive
					confirmLabel="Delete policy"
					confirmDisabled={props.saving}
					onCancel={() => {
						takeHybridFileWriteOverride();
						setConfirmDelete(false);
					}}
					onConfirm={() => {
						setConfirmDelete(false);
						props.onDisable();
					}}
				>
					<p>
						Delete the <strong>{props.title}</strong> policy from this configuration?
					</p>
				</ConfirmDialog>
			) : null}
		</>
	);
}

export function PolicyEditorBody(props: {
	formId?: string;
	policyKey: string;
	customEditor?: PolicyEditorKind;
	policyValue: unknown;
	help: SchemaHelp;
	saving: boolean;
	schemaRoot?: string;
	onSave: (value: unknown) => void;
}) {
	if (!props.customEditor) {
		return (
			<GenericPolicyEditor
				policyKey={props.policyKey}
				formId={props.formId}
				value={props.policyValue}
				help={props.help}
				saving={props.saving}
				schemaRoot={props.schemaRoot}
				onSave={props.onSave}
			/>
		);
	}
	return <CustomPolicyEditor {...props} customEditor={props.customEditor} />;
}

function CustomPolicyEditor(
	props: Parameters<typeof PolicyEditorBody>[0] & { customEditor: PolicyEditorKind }
) {
	const schemaRoot = props.schemaRoot ?? 'LocalLLMPolicy';
	const description = props.help.propertyDescription(schemaRoot, [props.policyKey]);
	return (
		<EditorSchemaContext.Provider value={['$defs', schemaRoot, 'properties', props.policyKey]}>
			<div className="policy-custom-editor">
				{description ? <p className="policy-schema-description">{description}</p> : null}
				{props.customEditor === 'accessLog' ? (
					<AccessLogPolicyEditor
						formId={props.formId}
						accessLog={props.policyValue as LoggingPolicy | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'authorization' ? (
					<AuthorizationPolicyEditor
						formId={props.formId}
						authorization={props.policyValue as AuthorizationDraft | null | undefined}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'backendAuth' ? (
					<BackendAuthPolicyEditor
						formId={props.formId}
						backendAuth={props.policyValue as BackendAuth | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'cors' ? (
					<CorsPolicyEditor
						formId={props.formId}
						cors={props.policyValue as CorsPolicy | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'extAuthz' ? (
					<ExtAuthzPolicyEditor
						formId={props.formId}
						extAuthz={props.policyValue as ExtAuthzDraft | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'extProc' ? (
					<ExtProcPolicyEditor
						formId={props.formId}
						extProc={props.policyValue as ExtProcDraft | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'jwtAuth' ? (
					<JwtPolicyEditor
						formId={props.formId}
						jwt={props.policyValue as JwtPolicy | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'localRateLimit' ? (
					<LocalRateLimitPolicyEditor
						formId={props.formId}
						localRateLimit={props.policyValue as LocalRateLimitConfig | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'mcpAuthentication' ? (
					<McpAuthenticationPolicyEditor
						formId={props.formId}
						authentication={props.policyValue as McpAuthenticationDraft | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'mcpAuthorization' ? (
					<AuthorizationPolicyEditor
						formId={props.formId}
						authorization={props.policyValue as AuthorizationDraft | null | undefined}
						newRuleExpression={'mcp.tool.name == "get_weather"'}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'mcpGuardrails' ? (
					<McpGuardrailsPolicyEditor
						formId={props.formId}
						guardrails={props.policyValue as McpGuardrailsDraft | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'oidc' ? (
					<OidcPolicyEditor
						formId={props.formId}
						oidc={props.policyValue as OidcDraft | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'remoteRateLimit' ? (
					<RemoteRateLimitPolicyEditor
						formId={props.formId}
						remoteRateLimit={props.policyValue as RemoteRateLimitDraft | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : props.customEditor === 'tracing' ? (
					<TracingPolicyEditor
						formId={props.formId}
						tracing={props.policyValue as TracingConfig | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				) : (
					<TransformationsPolicyEditor
						formId={props.formId}
						transformations={props.policyValue as TransformationDraft | null | undefined}
						help={props.help}
						saving={props.saving}
						onSave={props.onSave}
					/>
				)}
			</div>
		</EditorSchemaContext.Provider>
	);
}

// Raw mode for a drawer showing PolicyEditorBody: its editor reports its value
// (wrap the body in EditorValueContext with `report`), and raw edits reload the
// editor (key the body on `value`).
export function usePolicyRaw(props: {
	formId: string;
	policyValue: unknown;
	schemaPath: string[];
	onSave: (value: unknown) => void;
}) {
	const [value, setValue] = useState(props.policyValue);
	const reported = useRef<{ value: unknown } | null>(null);
	// No form: raw mode, or an editor that cannot show this value.
	const form = () => document.getElementById(props.formId) as HTMLFormElement | null;

	return {
		value,
		report(next: unknown) {
			reported.current = { value: next };
		},
		// Saves through the editor's form (and its validation), or, without one, the value.
		submit() {
			const current = form();
			if (current) current.requestSubmit();
			else props.onSave(value);
		},
		raw: {
			schemaPath: props.schemaPath,
			value() {
				if (!form() || !reported.current) return value;
				return reported.current.value;
			},
			// The editor unmounts in raw mode; this is the value it reloads from.
			onOpen: setValue,
			onChange: setValue,
			empty: null
		} satisfies DrawerRaw
	};
}

function sanitizePolicyFormId(value: string) {
	return value.replace(/[^A-Za-z0-9_-]+/g, '-');
}
