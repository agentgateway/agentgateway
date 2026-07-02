// Parent-level guards (full component swap, no conditional hooks — see
// xdsGuardedPages.tsx for rationale) wiring each LLM/MCP nav destination to
// a real read-only dump view when the gateway is XDS-managed, falling back
// to the original standalone-mode-only page otherwise.
import { StatusBanner } from "../components/Primitives";
import { useConfigDumpMode } from "../hooks";
import { ModelsPage } from "./Models";
import { ProvidersPage } from "./Providers";
import { GuardrailsPage } from "./Guardrails";
import { KeysPage } from "./Keys";
import { McpServersPage } from "./McpServers";
import { PoliciesPage, McpPoliciesPage } from "./Policies";
import { LlmBackendsDumpPage } from "./LlmBackendsDump";
import { McpBackendsDumpPage } from "./McpBackendsDump";
import { GuardrailsDumpPage } from "./GuardrailsDump";
import { VirtualKeysDumpPage } from "./VirtualKeysDump";
import { DumpPoliciesPage } from "./DumpPolicies";
import { McpPoliciesDumpPage } from "./McpPoliciesDump";

function useIsDumpMode() {
  const mode = useConfigDumpMode();
  return { isLoading: mode.isLoading, isDump: mode.data?.mode === "dump" };
}

export function GuardedModelsPage() {
  const { isLoading, isDump } = useIsDumpMode();
  if (isLoading) return <StatusBanner state="loading" title="Loading" />;
  return isDump ? <LlmBackendsDumpPage /> : <ModelsPage />;
}

export function GuardedProvidersPage() {
  const { isLoading, isDump } = useIsDumpMode();
  if (isLoading) return <StatusBanner state="loading" title="Loading" />;
  // Our runtime dump doesn't distinguish "providers" from "models" the way
  // standalone config does (each AI backend already carries its provider
  // endpoint) — the same read-only backends view serves both nav items.
  return isDump ? <LlmBackendsDumpPage /> : <ProvidersPage />;
}

export function GuardedLlmPoliciesPage() {
  const { isLoading, isDump } = useIsDumpMode();
  if (isLoading) return <StatusBanner state="loading" title="Loading" />;
  // Reuses the same generic, already-production-proven policy dump view
  // used by Traffic > Policies — agentgateway has one unified policy CRD,
  // so there's nothing LLM-specific to filter here.
  return isDump ? <DumpPoliciesPage /> : <PoliciesPage />;
}

export function GuardedGuardrailsPage() {
  const { isLoading, isDump } = useIsDumpMode();
  if (isLoading) return <StatusBanner state="loading" title="Loading" />;
  return isDump ? <GuardrailsDumpPage /> : <GuardrailsPage />;
}

export function GuardedKeysPage() {
  const { isLoading, isDump } = useIsDumpMode();
  if (isLoading) return <StatusBanner state="loading" title="Loading" />;
  return isDump ? <VirtualKeysDumpPage /> : <KeysPage />;
}

export function GuardedMcpServersPage() {
  const { isLoading, isDump } = useIsDumpMode();
  if (isLoading) return <StatusBanner state="loading" title="Loading" />;
  return isDump ? <McpBackendsDumpPage /> : <McpServersPage />;
}

export function GuardedMcpPoliciesPage() {
  const { isLoading, isDump } = useIsDumpMode();
  if (isLoading) return <StatusBanner state="loading" title="Loading" />;
  return isDump ? <McpPoliciesDumpPage /> : <McpPoliciesPage />;
}
