import { Save, Trash2 } from "lucide-react";
import { Drawer, StatusBanner, Tooltip } from "../components/Primitives";
import type { SchemaHelp } from "../schemaHelp";
import type { CorsPolicy } from "../types";
import { AuthorizationPolicyEditor } from "./AuthorizationPolicyEditor";
import { CorsPolicyEditor } from "./CorsPolicyEditor";
import { ExtAuthzPolicyEditor } from "./ExtAuthzPolicyEditor";
import { ExtProcPolicyEditor } from "./ExtProcPolicyEditor";
import { GenericPolicyEditor } from "./GenericPolicyEditor";
import { JwtPolicyEditor } from "./JwtPolicyEditor";
import { LocalRateLimitPolicyEditor } from "./LocalRateLimitPolicyEditor";
import { McpAuthenticationPolicyEditor } from "./McpAuthenticationPolicyEditor";
import { McpGuardrailsPolicyEditor } from "./McpGuardrailsPolicyEditor";
import { OidcPolicyEditor } from "./OidcPolicyEditor";
import { RemoteRateLimitPolicyEditor } from "./RemoteRateLimitPolicyEditor";
import { TransformationsPolicyEditor } from "./TransformationsPolicyEditor";
import { policyEnabled } from "./policyUtils";
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
  TransformationDraft,
} from "./types";

export type PolicyEditorKind =
  | "authorization"
  | "cors"
  | "extAuthz"
  | "extProc"
  | "jwtAuth"
  | "localRateLimit"
  | "mcpAuthentication"
  | "mcpAuthorization"
  | "mcpGuardrails"
  | "oidc"
  | "remoteRateLimit"
  | "transformations";

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
  onClose: () => void;
  onSave: (value: unknown) => void;
  onDisable: () => void;
}) {
  const enabled = policyEnabled(props.policies, props.policyKey);
  const formId = `policy-editor-${sanitizePolicyFormId(props.schemaRoot ?? "LocalLLMPolicy")}-${sanitizePolicyFormId(props.policyKey)}`;
  return (
    <Drawer
      title={props.title}
      onClose={props.onClose}
      headerActions={
        <Tooltip content={enabled ? "Delete policy" : "Policy is not enabled"}>
          <button
            className="icon-button danger"
            type="button"
            aria-label="Delete policy"
            disabled={!enabled || props.saving}
            onClick={props.onDisable}
          >
            <Trash2 size={17} />
          </button>
        </Tooltip>
      }
      footer={
        <button
          className="button primary"
          type="submit"
          form={formId}
          disabled={props.saving}
        >
          <Save size={16} />
          Save
        </button>
      }
    >
      <PolicyEditorBody
        formId={formId}
        policyKey={props.policyKey}
        customEditor={props.customEditor}
        policyValue={props.policyValue}
        help={props.help}
        saving={props.saving}
        schemaRoot={props.schemaRoot}
        onSave={props.onSave}
      />
      {props.saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {props.saveError}
        </StatusBanner>
      ) : null}
    </Drawer>
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
  showGenericSchemaDescription?: boolean;
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
        showSchemaDescription={props.showGenericSchemaDescription}
        onSave={props.onSave}
      />
    );
  }
  const description = props.help.propertyDescription(
    props.schemaRoot ?? "LocalLLMPolicy",
    [props.policyKey],
  );
  return (
    <div className="policy-custom-editor">
      {description ? (
        <p className="policy-schema-description">{description}</p>
      ) : null}
      {props.customEditor === "authorization" ? (
        <AuthorizationPolicyEditor
          formId={props.formId}
          authorization={
            props.policyValue as AuthorizationDraft | null | undefined
          }
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "cors" ? (
        <CorsPolicyEditor
          formId={props.formId}
          cors={props.policyValue as CorsPolicy | null | undefined}
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "extAuthz" ? (
        <ExtAuthzPolicyEditor
          formId={props.formId}
          extAuthz={props.policyValue as ExtAuthzDraft | null | undefined}
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "extProc" ? (
        <ExtProcPolicyEditor
          formId={props.formId}
          extProc={props.policyValue as ExtProcDraft | null | undefined}
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "jwtAuth" ? (
        <JwtPolicyEditor
          formId={props.formId}
          jwt={props.policyValue as JwtPolicy | null | undefined}
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "localRateLimit" ? (
        <LocalRateLimitPolicyEditor
          formId={props.formId}
          localRateLimit={
            props.policyValue as LocalRateLimitConfig | null | undefined
          }
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "mcpAuthentication" ? (
        <McpAuthenticationPolicyEditor
          formId={props.formId}
          authentication={
            props.policyValue as McpAuthenticationDraft | null | undefined
          }
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "mcpAuthorization" ? (
        <AuthorizationPolicyEditor
          formId={props.formId}
          authorization={
            props.policyValue as AuthorizationDraft | null | undefined
          }
          newRuleExpression={'mcp.tool.name == "get_weather"'}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "mcpGuardrails" ? (
        <McpGuardrailsPolicyEditor
          formId={props.formId}
          guardrails={
            props.policyValue as McpGuardrailsDraft | null | undefined
          }
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "oidc" ? (
        <OidcPolicyEditor
          formId={props.formId}
          oidc={props.policyValue as OidcDraft | null | undefined}
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : props.customEditor === "remoteRateLimit" ? (
        <RemoteRateLimitPolicyEditor
          formId={props.formId}
          remoteRateLimit={
            props.policyValue as RemoteRateLimitDraft | null | undefined
          }
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      ) : (
        <TransformationsPolicyEditor
          formId={props.formId}
          transformations={
            props.policyValue as TransformationDraft | null | undefined
          }
          help={props.help}
          saving={props.saving}
          onSave={props.onSave}
        />
      )}
    </div>
  );
}

function sanitizePolicyFormId(value: string) {
  return value.replace(/[^A-Za-z0-9_-]+/g, "-");
}
