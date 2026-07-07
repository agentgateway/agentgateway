import { useMemo } from "react";
import {
  Dropdown,
  FieldGroup,
  Panel,
  StatusBanner,
} from "../components/Primitives";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import type { LocalUIConfig } from "../gateway-config";
import type { GatewayConfig, TrafficGateway } from "../types";
import type { PolicyKey } from "../policies/types";
import { PolicyCatalogPage } from "./Policies";

const uiPolicySections: Array<{ title: string; keys: PolicyKey[] }> = [
  {
    title: "Access",
    keys: [
      "oidc",
      "jwtAuth",
      "authorization",
      "extAuthz",
      "basicAuth",
      "apiKey",
      "csrf",
      "cors",
    ] as PolicyKey[],
  },
];

export function RawSettingsPage() {
  return (
    <PolicyCatalogPage
      title="Top-level Settings"
      description="Configure UI exposure and UI policies."
      schemaRoot="LocalUIPolicy"
      sections={uiPolicySections}
      yamlDescription="Read-only view of ui.policies."
      policies={(config) =>
        config.data?.ui?.policies as Record<string, unknown> | null | undefined
      }
      beforePolicies={<UiGatewayPanel />}
      onSavePolicy={(next, key, value) => {
        next.ui ??= {};
        next.ui.policies ??= {};
        (next.ui.policies as Record<string, unknown>)[key] = value;
      }}
      onDisablePolicy={(next, key) => {
        if (next.ui?.policies) {
          delete (next.ui.policies as Record<string, unknown>)[key];
          if (Object.keys(next.ui.policies).length === 0) {
            delete next.ui.policies;
          }
        }
      }}
    />
  );
}

function UiGatewayPanel() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const gatewayOptions = useMemo(
    () => gatewayReferenceOptions(config.data),
    [config.data],
  );
  const initialGateway = useMemo(
    () =>
      firstGatewayRef(config.data?.ui?.gateways) ?? gatewayOptions[0]?.value,
    [config.data?.ui?.gateways, gatewayOptions],
  );

  function setGateway(gateway: string) {
    update.mutate((next) => {
      next.ui ??= {};
      if (gateway) next.ui.gateways = gateway;
      else delete next.ui.gateways;
    });
  }

  return (
    <Panel>
      <div className="form-grid">
        <FieldGroup
          label="UI gateway"
          tooltip="Gateway reference used to expose the UI and required UI API routes."
        >
          <Dropdown
            ariaLabel="UI gateway"
            value={initialGateway ?? ""}
            options={gatewayOptions}
            disabled={!gatewayOptions.length || update.isPending}
            placeholder="No gateways configured"
            allowEmpty
            onChange={setGateway}
          />
        </FieldGroup>
      </div>
      {!gatewayOptions.length ? (
        <StatusBanner state="warn" title="No gateways configured">
          Add a gateway before exposing the UI.
        </StatusBanner>
      ) : null}
      {update.isError ? (
        <StatusBanner state="bad" title="Save failed">
          {update.error.message}
        </StatusBanner>
      ) : null}
      {update.isSuccess ? (
        <StatusBanner state="ok" title="Gateway saved" />
      ) : null}
    </Panel>
  );
}

function gatewayReferenceOptions(config: GatewayConfig | null | undefined) {
  return Object.entries(config?.gateways ?? {}).flatMap(([name, gateway]) => {
    const listeners = gateway.listeners ?? [];
    if (!listeners.length) {
      return [
        {
          value: name,
          label: name,
          description: gateway.port ? `Port ${gateway.port}` : undefined,
        },
      ];
    }
    return [
      {
        value: name,
        label: `${name} (all listeners)`,
        description: gatewayDescription(gateway),
      },
      ...listeners.map((listener, index) => {
        const listenerName = listener.name ?? `listener${index}`;
        return {
          value: `${name}/${listenerName}`,
          label: `${name}/${listenerName}`,
          description: listener.hostname || gatewayDescription(gateway),
        };
      }),
    ];
  });
}

function gatewayDescription(gateway: TrafficGateway) {
  return gateway.port ? `Port ${gateway.port}` : undefined;
}

function firstGatewayRef(gateways: LocalUIConfig["gateways"] | undefined) {
  if (Array.isArray(gateways)) return gateways[0];
  return gateways;
}
