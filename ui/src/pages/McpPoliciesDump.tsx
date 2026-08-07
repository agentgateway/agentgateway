import { Eye, ShieldCheck } from "lucide-react";
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
  policyName,
  policyTargetLabel,
  policyTypeLabel,
  type TargetedPolicy,
} from "./policyDumpUtils";

function isMcpTargeted(policy: TargetedPolicy) {
  const target = policyTargetLabel(policy.target).toLowerCase();
  return target.includes("mcp") || policy.key.toLowerCase().includes("mcp");
}

export function McpPoliciesDumpPage() {
  const mode = useConfigDumpMode();
  const [selectedKey, setSelectedKey] = useStickyQueryParam("mcp-policy");
  const dumpMode = mode.data?.mode === "dump";
  const allPolicies = (mode.data?.dump?.policies ?? []).filter(
    isTargetedPolicy,
  );
  const policies = allPolicies.filter(isMcpTargeted);
  const selectedPolicy = policies.find((policy) => policy.key === selectedKey);

  return (
    <div className="page-stack">
      <PageHeader
        title="MCP Policies"
        description="Read-only policies targeting MCP backends/routes from the active gateway dump."
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
          <StatusBanner state="warn" title="Readonly policies unavailable">
            MCP policies are only viewable here when the gateway is running
            from XDS config.
          </StatusBanner>
        ) : !policies.length ? (
          <EmptyState
            title="No MCP-targeted policies"
            description={`No policies targeting an MCP backend/route were found among ${allPolicies.length} total policies in the active dump — this gateway currently has no MCP backends configured (see MCP > Servers). Any AgentgatewayPolicy targeting an MCP backend will appear here once one exists.`}
          />
        ) : (
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
                {policies.map((policy) => (
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
                          onClick={() => setSelectedKey(policy.key)}
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
        )}
      </Panel>

      {selectedPolicy ? (
        <Drawer
          title={policyName(selectedPolicy)}
          headerActions={
            <span className="badge">
              <ShieldCheck size={14} /> {policyTypeLabel(selectedPolicy.policy)}
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
