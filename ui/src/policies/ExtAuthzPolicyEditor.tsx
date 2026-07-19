import { tr } from "../i18n";
import { ShieldCheck } from "lucide-react";
import { useState } from "react";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { EnumSelector } from "../components/EnumSelector";
import { UnsupportedYamlFallback } from "../components/EditorContracts";
import { Field, FieldGroup } from "../components/Primitives";
import { ListEditor } from "./ListEditor";
import {
  hasUnsupportedTarget,
  KeyValueEditor,
  TargetEditor,
  targetFrom,
  unsupportedTargetLabel,
} from "./PolicyFormControls";
import { PolicySection } from "./PolicyLayout";
import { ResultingYaml } from "./ResultingYaml";
import { cleanEmpty } from "./policyUtils";
import type { SchemaHelp } from "../schemaHelp";
import type { ExtAuthzDraft } from "./types";
import type { BodyOptions, ExtAuthz } from "../gateway-config";

type ProtocolMode = "grpc" | "http";
type FailureMode = "deny" | "allow" | "denyWithStatus";

export function ExtAuthzPolicyEditor(props: {
  formId?: string;
  extAuthz: ExtAuthzDraft | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (value: ExtAuthzDraft) => void;
}) {
  const unsupportedTarget = hasUnsupportedTarget(props.extAuthz);
  const [target, setTarget] = useState(() => targetFrom(props.extAuthz));
  const [protocolMode, setProtocolMode] = useState<ProtocolMode>(
    props.extAuthz?.protocol && "http" in props.extAuthz.protocol
      ? "http"
      : "grpc",
  );
  const [failureMode, setFailureMode] = useState<FailureMode>(
    failureModeFrom(props.extAuthz?.failureMode),
  );
  const [denyStatus, setDenyStatus] = useState(
    typeof props.extAuthz?.failureMode === "object"
      ? props.extAuthz.failureMode.denyWithStatus
      : 403,
  );
  const [includeRequestHeaders, setIncludeRequestHeaders] = useState(
    props.extAuthz?.includeRequestHeaders ?? [],
  );
  const [includeBody, setIncludeBody] = useState(
    Boolean(props.extAuthz?.includeRequestBody),
  );
  const [maxRequestBytes, setMaxRequestBytes] = useState(
    props.extAuthz?.includeRequestBody?.maxRequestBytes ?? "",
  );
  const [allowPartialMessage, setAllowPartialMessage] = useState(
    Boolean(props.extAuthz?.includeRequestBody?.allowPartialMessage),
  );
  const [packAsBytes, setPackAsBytes] = useState(
    Boolean(props.extAuthz?.includeRequestBody?.packAsBytes),
  );
  const grpc =
    props.extAuthz?.protocol && "grpc" in props.extAuthz.protocol
      ? props.extAuthz.protocol.grpc
      : {};
  const http =
    props.extAuthz?.protocol && "http" in props.extAuthz.protocol
      ? props.extAuthz.protocol.http
      : {};
  const [grpcContext, setGrpcContext] = useState(grpc.context ?? {});
  const [grpcMetadata, setGrpcMetadata] = useState(grpc.metadata ?? {});
  const [httpPath, setHttpPath] = useState(http.path ?? "");
  const [httpRedirect, setHttpRedirect] = useState(http.redirect ?? "");
  const [includeResponseHeaders, setIncludeResponseHeaders] = useState(
    http.includeResponseHeaders ?? [],
  );
  const [addRequestHeaders, setAddRequestHeaders] = useState(
    http.addRequestHeaders ?? {},
  );
  const [httpMetadata, setHttpMetadata] = useState(http.metadata ?? {});
  const preview = buildExtAuthz();

  if (unsupportedTarget) {
    return (
      <UnsupportedYamlFallback
        title={tr("copy.unsupportedTargetType")}
        value={props.extAuthz ?? {}}
        schema={props.help.node(["$defs", "ExtAuthz"])}
        help={props.help}
      >
        {tr("copy.thisPolicyUsesA")}
        {unsupportedTargetLabel(props.extAuthz)}
        {tr("copy.targetTheVisualEditorCurrentlySupportsHostTargetsOnly")}
      </UnsupportedYamlFallback>
    );
  }

  function buildExtAuthz() {
    return cleanEmpty({
      ...target,
      failureMode:
        failureMode === "denyWithStatus"
          ? { denyWithStatus: denyStatus }
          : failureMode,
      includeRequestHeaders,
      includeRequestBody: includeBody
        ? {
            maxRequestBytes:
              maxRequestBytes === "" ? undefined : Number(maxRequestBytes),
            allowPartialMessage: allowPartialMessage ? true : undefined,
            packAsBytes: packAsBytes ? true : undefined,
          }
        : undefined,
      protocol:
        protocolMode === "grpc"
          ? {
              grpc: {
                context: grpcContext,
                metadata: grpcMetadata,
              },
            }
          : {
              http: {
                path: httpPath,
                redirect: httpRedirect,
                includeResponseHeaders,
                addRequestHeaders,
                metadata: httpMetadata,
              },
            },
    }) as ExtAuthzDraft;
  }

  return (
    <form
      id={props.formId}
      className="policy-editor-stack"
      onSubmit={(event) => {
        event.preventDefault();
        props.onSave(preview);
      }}
    >
      <TargetEditor
        value={target}
        tooltip={props.help.field<ExtAuthz>("ExtAuthz", "host")}
        onChange={setTarget}
      />
      <PolicySection
        icon={<ShieldCheck size={17} />}
        title={tr("copy.authorizationBehavior")}
        description={tr("copy.chooseProtocolAndFailOpenFailClosedBehavior")}
      >
        <div className="form-grid">
          <FieldGroup
            label={tr("copy.protocol")}
            tooltip={props.help.field<ExtAuthz>("ExtAuthz", "protocol")}
          >
            <EnumSelector
              ariaLabel="Protocol"
              value={protocolMode}
              options={[
                {
                  value: "grpc",
                  label: "gRPC",
                  description: tr("copy.useEnvoyExternalAuthorizationOverGRpc"),
                },
                {
                  value: "http",
                  label: "HTTP",
                  description: tr("copy.callAnHttpAuthorizationService"),
                },
              ]}
              onChange={setProtocolMode}
            />
          </FieldGroup>
          <FieldGroup
            label={tr("copy.failureMode")}
            tooltip={props.help.field<ExtAuthz>("ExtAuthz", "failureMode")}
          >
            <EnumSelector
              ariaLabel="Failure mode"
              value={failureMode}
              options={[
                { value: "deny", label: tr("copy.deny") },
                { value: "allow", label: tr("copy.allow") },
                { value: "denyWithStatus", label: tr("copy.denyWithStatus") },
              ]}
              schema={props.help.node(["$defs", "FailureMode4"])}
              onChange={setFailureMode}
            />
          </FieldGroup>
        </div>
        {failureMode === "denyWithStatus" ? (
          <Field
            label={tr("copy.denyStatus")}
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "failureMode.denyWithStatus",
            )}
          >
            <input
              type="number"
              value={denyStatus}
              onChange={(event) => setDenyStatus(Number(event.target.value))}
            />
          </Field>
        ) : null}
        <ListEditor
          label={tr("copy.includeRequestHeaders")}
          tooltip={props.help.field<ExtAuthz>(
            "ExtAuthz",
            "includeRequestHeaders",
          )}
          values={includeRequestHeaders}
          placeholder="authorization"
          onChange={setIncludeRequestHeaders}
        />
        <label className="config-option-row">
          <input
            type="checkbox"
            checked={includeBody}
            onChange={(event) => setIncludeBody(event.target.checked)}
          />
          <span>
            <strong>{tr("copy.includeRequestBody")}</strong>
            <small>
              {props.help.field<ExtAuthz>("ExtAuthz", "includeRequestBody")}
            </small>
          </span>
        </label>
        {includeBody ? (
          <div className="form-grid">
            <Field
              label={tr("copy.maxRequestBytes")}
              tooltip={props.help.field<BodyOptions>(
                "BodyOptions",
                "maxRequestBytes",
              )}
            >
              <input
                type="number"
                value={maxRequestBytes}
                onChange={(event) =>
                  setMaxRequestBytes(
                    event.target.value === "" ? "" : Number(event.target.value),
                  )
                }
                placeholder="8192"
              />
            </Field>
            <FieldGroup
              label={tr("copy.bodyOptions")}
              tooltip={props.help.field<ExtAuthz>(
                "ExtAuthz",
                "includeRequestBody",
              )}
            >
              <label className="config-option-row">
                <input
                  type="checkbox"
                  checked={allowPartialMessage}
                  onChange={(event) =>
                    setAllowPartialMessage(event.target.checked)
                  }
                />
                <span>
                  <strong>{tr("copy.allowPartialMessage")}</strong>
                  <small>
                    {props.help.field<BodyOptions>(
                      "BodyOptions",
                      "allowPartialMessage",
                    )}
                  </small>
                </span>
              </label>
              <label className="config-option-row">
                <input
                  type="checkbox"
                  checked={packAsBytes}
                  onChange={(event) => setPackAsBytes(event.target.checked)}
                />
                <span>
                  <strong>{tr("copy.packAsBytes")}</strong>
                  <small>
                    {props.help.field<BodyOptions>(
                      "BodyOptions",
                      "packAsBytes",
                    )}
                  </small>
                </span>
              </label>
            </FieldGroup>
          </div>
        ) : null}
      </PolicySection>

      {protocolMode === "grpc" ? (
        <PolicySection
          icon={<ShieldCheck size={17} />}
          title={tr("copy.gRpcDetails")}
          description={tr(
            "copy.contextExtensionsAreStaticValuesMetadataValuesAreCelExpressions",
          )}
        >
          <KeyValueEditor
            label={tr("copy.context")}
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "protocol.grpc.context",
            )}
            values={grpcContext}
            keyPlaceholder="key"
            valuePlaceholder="value"
            onChange={setGrpcContext}
          />
          <KeyValueEditor
            label={tr("copy.metadata")}
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "protocol.grpc.metadata",
            )}
            values={grpcMetadata}
            keyPlaceholder="key"
            valuePlaceholder="CEL expression"
            valueKind="cel"
            onChange={setGrpcMetadata}
          />
        </PolicySection>
      ) : (
        <PolicySection
          icon={<ShieldCheck size={17} />}
          title={tr("copy.httpDetails")}
          description={tr(
            "copy.configureTheAuthorizationRequestAndResponseMetadataExtraction",
          )}
        >
          <FieldGroup
            label={tr("copy.pathExpression")}
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "protocol.http.path",
            )}
          >
            <MiniMonacoEditor
              className="micro"
              language="cel"
              value={httpPath}
              onChange={setHttpPath}
              placeholder={tr("copy.oauth2Auth")}
            />
          </FieldGroup>
          <FieldGroup
            label={tr("copy.redirectExpression")}
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "protocol.http.redirect",
            )}
          >
            <MiniMonacoEditor
              className="micro"
              language="cel"
              value={httpRedirect}
              onChange={setHttpRedirect}
              placeholder={'"/oauth2/start?rd=" + request.path'}
            />
          </FieldGroup>
          <ListEditor
            label={tr("copy.includeResponseHeaders")}
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "protocol.http.includeResponseHeaders",
            )}
            values={includeResponseHeaders}
            placeholder="x-auth-request-user"
            onChange={setIncludeResponseHeaders}
          />
          <KeyValueEditor
            label={tr("copy.addRequestHeaders")}
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "protocol.http.addRequestHeaders",
            )}
            values={addRequestHeaders}
            keyPlaceholder="x-forwarded-host"
            valuePlaceholder="request.host"
            valueKind="cel"
            onChange={setAddRequestHeaders}
          />
          <KeyValueEditor
            label={tr("copy.metadata")}
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "protocol.http.metadata",
            )}
            values={httpMetadata}
            keyPlaceholder="user"
            valuePlaceholder={'response.headers["x-auth-request-user"]'}
            valueKind="cel"
            onChange={setHttpMetadata}
          />
        </PolicySection>
      )}

      <ResultingYaml value={preview} />
    </form>
  );
}

function failureModeFrom(
  value: ExtAuthzDraft["failureMode"] | undefined,
): FailureMode {
  if (value === "allow" || value === "deny") return value;
  if (value && typeof value === "object") return "denyWithStatus";
  return "deny";
}
