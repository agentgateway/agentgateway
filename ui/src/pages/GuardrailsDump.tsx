import { Eye, Shield } from "lucide-react";
import {
  Drawer,
  EmptyState,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../components/Primitives";
import { useStickyQueryParam } from "../drawerRouteState";
import { useConfigDumpMode } from "../hooks";
import { ReadonlyModeBanner } from "./traffic/TrafficConfigDumpPanel";
import {
  isTargetedPolicy,
  policyInheritanceLabel,
  policyMatchesKeywords,
  policyName,
  policyTargetLabel,
  policyTypeLabel,
  type TargetedPolicy,
} from "./policyDumpUtils";

const GUARDRAIL_KEYWORDS = ["guard", "moderation", "modelarmor"];

export function GuardrailsDumpPage() {
  const mode = useConfigDumpMode();
  const [selectedKey, setSelectedKey] = useStickyQueryParam("guardrail");
  const dumpMode = mode.data?.mode === "dump";
  const allPolicies = (mode.data?.dump?.policies ?? []).filter(
    isTargetedPolicy,
  );
  const policies = allPolicies.filter((p) =>
    policyMatchesKeywords(p, GUARDRAIL_KEYWORDS),
  );
  const selectedPolicy = policies.find((policy) => policy.key === selectedKey);

  return (
    <div className="page-stack">
      <PageHeader
        title="Guardrails"
        description="Read-only view of prompt-guard / content-moderation policies from the active gateway dump."
      />
      <ReadonlyModeBanner />

      <Panel>
        {mode.isLoading ? (
          <StatusBanner state="loading" title="Loading runtime policies" />
        ) : mode.error ? (
          <StatusBanner state="bad" title="Config dump unavailable">
            {mode.error.message}
          </StatusBanner>
        ) : !dumpMode ? (
          <StatusBanner state="warn" title="Readonly guardrails unavailable">
            Guardrail policies are only viewable here when the gateway is
            running from XDS config.
          </StatusBanner>
        ) : !policies.length ? (
          <EmptyState
            title="No guardrail policies configured"
            description={`No policies matching a prompt-guard/moderation type were found among ${allPolicies.length} total policies in the active dump. Guardrails are configured via an AgentgatewayPolicy resource's backend.ai.promptGuard field (regex, webhook, OpenAI Moderations, AWS Bedrock Guardrails, or Google Model Armor) — none has been created yet.`}
          />
        ) : (
          <GuardrailsTable policies={policies} onSelect={setSelectedKey} />
        )}
      </Panel>

      {selectedPolicy ? (
        <Drawer
          title={policyName(selectedPolicy)}
          headerActions={
            <span className="badge">
              <Shield size={14} /> {policyTypeLabel(selectedPolicy.policy)}
            </span>
          }
          onClose={() => setSelectedKey(null)}
        >
          <YamlBlock value={selectedPolicy} />
        </Drawer>
      ) : null}
    </div>
  );
}

function GuardrailsTable(props: {
  policies: TargetedPolicy[];
  onSelect: (key: string) => void;
}) {
  return (
    <div className="table-wrap">
      <table className="dump-policies-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Target</th>
            <th>Type</th>
            <th>Inheritance</th>
            <th aria-label="Actions" />
          </tr>
        </thead>
        <tbody>
          {props.policies.map((policy) => (
            <tr key={policy.key}>
              <td>
                <div className="resource-name-cell">
                  <strong>{policyName(policy)}</strong>
                  <small>{policy.name?.kind ?? "Policy"}</small>
                </div>
              </td>
              <td>{policyTargetLabel(policy.target)}</td>
              <td>
                <span className="badge">
                  {policyTypeLabel(policy.policy)}
                </span>
              </td>
              <td>{policyInheritanceLabel(policy.inheritance)}</td>
              <td className="row-actions">
                <Tooltip content="View policy">
                  <button
                    className="icon-button"
                    type="button"
                    aria-label={`View ${policyName(policy)}`}
                    onClick={() => props.onSelect(policy.key)}
                  >
                    <Eye size={16} />
                  </button>
                </Tooltip>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
