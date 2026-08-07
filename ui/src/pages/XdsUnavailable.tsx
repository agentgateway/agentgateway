import type { ReactNode } from "react";
import { EmptyState, PageHeader, Panel } from "../components/Primitives";
import { ReadonlyModeBanner } from "./traffic/TrafficConfigDumpPanel";

/**
 * Shared placeholder for LLM/MCP UI pages that fundamentally cannot be
 * sourced from the read-only XDS config dump, because the data they show
 * (local prompt/response logs, in-UI cost bookkeeping, generated client
 * snippets) was never part of gateway *config* to begin with — it lived in
 * the standalone binary's own local state, which doesn't exist when the
 * gateway is Kubernetes/XDS-managed. Rather than silently rendering broken
 * (empty query, misleading UI), this explains why and points at the real
 * equivalent for this deployment.
 */
export function XdsUnavailablePage(props: {
  title: string;
  description: string;
  reason: string;
  alternative?: ReactNode;
}) {
  return (
    <div className="page-stack">
      <PageHeader title={props.title} description={props.description} />
      <ReadonlyModeBanner />
      <Panel>
        <EmptyState
          title="Not available for this deployment"
          description={props.reason}
          action={props.alternative}
        />
      </Panel>
    </div>
  );
}
