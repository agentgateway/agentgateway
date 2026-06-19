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
        title="Unsupported target type"
        value={props.extAuthz ?? {}}
        schema={props.help.node(["$defs", "ExtAuthz"])}
        help={props.help}
      >
        This policy uses a {unsupportedTargetLabel(props.extAuthz)} target. The
        visual editor currently supports host targets only.
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
        title="Authorization behavior"
        description="Choose protocol and fail-open/fail-closed behavior."
      >
        <div className="form-grid">
          <FieldGroup
            label="Protocol"
            tooltip={props.help.field<ExtAuthz>("ExtAuthz", "protocol")}
          >
            <EnumSelector
              ariaLabel="Protocol"
              value={protocolMode}
              options={[
                {
                  value: "grpc",
                  label: "gRPC",
                  description: "Use Envoy external authorization over gRPC.",
                },
                {
                  value: "http",
                  label: "HTTP",
                  description: "Call an HTTP authorization service.",
                },
              ]}
              onChange={setProtocolMode}
            />
          </FieldGroup>
          <FieldGroup
            label="Failure mode"
            tooltip={props.help.field<ExtAuthz>("ExtAuthz", "failureMode")}
          >
            <EnumSelector
              ariaLabel="Failure mode"
              value={failureMode}
              options={[
                { value: "deny", label: "Deny" },
                { value: "allow", label: "Allow" },
                { value: "denyWithStatus", label: "Deny with status" },
              ]}
              schema={props.help.node(["$defs", "FailureMode4"])}
              onChange={setFailureMode}
            />
          </FieldGroup>
        </div>
        {failureMode === "denyWithStatus" ? (
          <Field
            label="Deny status"
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
          label="Include request headers"
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
            <strong>Include request body</strong>
            <small>
              {props.help.field<ExtAuthz>("ExtAuthz", "includeRequestBody")}
            </small>
          </span>
        </label>
        {includeBody ? (
          <div className="form-grid">
            <Field
              label="Max request bytes"
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
              label="Body options"
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
                  <strong>Allow partial message</strong>
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
                  <strong>Pack as bytes</strong>
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
          title="gRPC details"
          description="Context extensions are static values; metadata values are CEL expressions."
        >
          <KeyValueEditor
            label="Context"
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
            label="Metadata"
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
          title="HTTP details"
          description="Configure the authorization request and response metadata extraction."
        >
          <FieldGroup
            label="Path expression"
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
              placeholder={'"/oauth2/auth"'}
            />
          </FieldGroup>
          <FieldGroup
            label="Redirect expression"
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
            label="Include response headers"
            tooltip={props.help.field<ExtAuthz>(
              "ExtAuthz",
              "protocol.http.includeResponseHeaders",
            )}
            values={includeResponseHeaders}
            placeholder="x-auth-request-user"
            onChange={setIncludeResponseHeaders}
          />
          <KeyValueEditor
            label="Add request headers"
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
            label="Metadata"
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
