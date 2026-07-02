import { Eye, KeyRound } from "lucide-react";
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

const KEY_KEYWORDS = ["apikey", "api_key", "virtualkey"];

export function VirtualKeysDumpPage() {
  const mode = useConfigDumpMode();
  const [selectedKey, setSelectedKey] = useStickyQueryParam("apikey");
  const dumpMode = mode.data?.mode === "dump";
  const allPolicies = (mode.data?.dump?.policies ?? []).filter(
    isTargetedPolicy,
  );
  const policies = allPolicies.filter((p) =>
    policyMatchesKeywords(p, KEY_KEYWORDS),
  );
  const selectedPolicy = policies.find((policy) => policy.key === selectedKey);

  return (
    <div className="page-stack">
      <PageHeader
        title="Virtual API Keys"
        description="Read-only view of API-key-authentication policies from the active gateway dump."
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
          <StatusBanner state="warn" title="Readonly keys unavailable">
            API key policies are only viewable here when the gateway is
            running from XDS config.
          </StatusBanner>
        ) : !policies.length ? (
          <EmptyState
            title="No API key policies configured"
            description={`No policies matching an API-key-authentication type were found among ${allPolicies.length} total policies in the active dump. This gateway currently authenticates via the WSO2 JWT policy (see Traffic > Policies); API key auth is configured via AgentgatewayPolicy's apiKeyAuthentication field and has not been created.`}
          />
        ) : (
          <KeysTable policies={policies} onSelect={setSelectedKey} />
        )}
      </Panel>

      {selectedPolicy ? (
        <Drawer
          title={policyName(selectedPolicy)}
          headerActions={
            <span className="badge">
              <KeyRound size={14} /> {policyTypeLabel(selectedPolicy.policy)}
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

function KeysTable(props: {
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
