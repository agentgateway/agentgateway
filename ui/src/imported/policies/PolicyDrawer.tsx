import { Drawer, StatusBanner } from "../components/Primitives";
import type { SchemaHelp } from "../schemaHelp";
import type { CorsPolicy } from "../types";
import { AuthorizationPolicyEditor } from "./AuthorizationPolicyEditor";
import { CorsPolicyEditor } from "./CorsPolicyEditor";
import { ExtAuthzPolicyEditor } from "./ExtAuthzPolicyEditor";
import { ExtProcPolicyEditor } from "./ExtProcPolicyEditor";
import { GenericPolicyEditor } from "./GenericPolicyEditor";
import { JwtPolicyEditor } from "./JwtPolicyEditor";
import { LocalRateLimitPolicyEditor } from "./LocalRateLimitPolicyEditor";
import { OidcPolicyEditor } from "./OidcPolicyEditor";
import { TransformationsPolicyEditor } from "./TransformationsPolicyEditor";
import { policyEnabled } from "./policyUtils";
import type { AuthorizationDraft, ExtAuthzDraft, ExtProcDraft, JwtPolicy, LocalRateLimitConfig, OidcDraft, TransformationDraft } from "./types";

export type PolicyEditorKind =
  | "authorization"
  | "cors"
  | "extAuthz"
  | "extProc"
  | "jwtAuth"
  | "localRateLimit"
  | "oidc"
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
  return (
    <Drawer
      title={props.title}
      onClose={props.onClose}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onClose}>Cancel</button>
          <button className="button danger" type="button" disabled={!enabled || props.saving} onClick={props.onDisable}>Disable</button>
        </div>
      }
    >
      <PolicyEditorBody
        policyKey={props.policyKey}
        customEditor={props.customEditor}
        policyValue={props.policyValue}
        help={props.help}
        saving={props.saving}
        schemaRoot={props.schemaRoot}
        onSave={props.onSave}
      />
      {props.saveError ? <StatusBanner state="bad" title="Save failed">{props.saveError}</StatusBanner> : null}
    </Drawer>
  );
}

export function PolicyEditorBody(props: {
  policyKey: string;
  customEditor?: PolicyEditorKind;
  policyValue: unknown;
  help: SchemaHelp;
  saving: boolean;
  schemaRoot?: string;
  onSave: (value: unknown) => void;
}) {
  return props.customEditor === "authorization" ? (
    <AuthorizationPolicyEditor authorization={props.policyValue as AuthorizationDraft | null | undefined} saving={props.saving} onSave={props.onSave} />
  ) : props.customEditor === "cors" ? (
    <CorsPolicyEditor cors={props.policyValue as CorsPolicy | null | undefined} help={props.help} saving={props.saving} onSave={props.onSave} />
  ) : props.customEditor === "extAuthz" ? (
    <ExtAuthzPolicyEditor extAuthz={props.policyValue as ExtAuthzDraft | null | undefined} help={props.help} saving={props.saving} onSave={props.onSave} />
  ) : props.customEditor === "extProc" ? (
    <ExtProcPolicyEditor extProc={props.policyValue as ExtProcDraft | null | undefined} help={props.help} saving={props.saving} onSave={props.onSave} />
  ) : props.customEditor === "jwtAuth" ? (
    <JwtPolicyEditor jwt={props.policyValue as JwtPolicy | null | undefined} help={props.help} saving={props.saving} onSave={props.onSave} />
  ) : props.customEditor === "localRateLimit" ? (
    <LocalRateLimitPolicyEditor localRateLimit={props.policyValue as LocalRateLimitConfig | null | undefined} help={props.help} saving={props.saving} onSave={props.onSave} />
  ) : props.customEditor === "oidc" ? (
    <OidcPolicyEditor oidc={props.policyValue as OidcDraft | null | undefined} help={props.help} saving={props.saving} onSave={props.onSave} />
  ) : props.customEditor === "transformations" ? (
    <TransformationsPolicyEditor transformations={props.policyValue as TransformationDraft | null | undefined} help={props.help} saving={props.saving} onSave={props.onSave} />
  ) : (
    <GenericPolicyEditor
      policyKey={props.policyKey}
      value={props.policyValue}
      help={props.help}
      saving={props.saving}
      schemaRoot={props.schemaRoot}
      onSave={props.onSave}
    />
  );
}
