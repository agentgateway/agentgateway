import { useEffect, useMemo, useState } from "react";
import {
  Dropdown,
  FieldGroup,
  Panel,
  StatusBanner,
} from "../components/Primitives";
import { ConfigDiffSaveActions } from "../components/ConfigDiffDrawer";
import { useEffectiveGatewayConfig, useUpdateConfig } from "../hooks";
import type { LocalUIConfig } from "../gateway-config";
import type { GatewayConfig, TrafficGateway } from "../types";
import type { PolicyKey } from "../policies/types";
import { PolicyCatalogPage } from "./Policies";

const noneGateway = "__none__";

const uiPolicySections: Array<{ title: string; keys: PolicyKey[] }> = [
  {
    title: "UI access policies",
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
      title="UI Settings"
      description="Expose the UI on a traffic gateway and configure policies that protect the UI."
      schemaRoot="LocalUIPolicy"
      resourceKind="ui.policy"
      sections={uiPolicySections}
      yamlDescription="Read-only view of effective UI policies, including database-backed resources in hybrid mode."
      policies={(config) =>
        config.data?.ui?.policies as Record<string, unknown> | null | undefined
      }
      policiesDisabled={(config) => !uiGateway(config.data)}
      policiesDisabledReason="UI policies require the UI to be exposed on a gateway."
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
  const config = useEffectiveGatewayConfig();
  const update = useUpdateConfig();
  const gatewayOptions = useMemo(
    () => gatewayReferenceOptions(config.data),
    [config.data],
  );
  const selectedGateway = uiGateway(config.data);
  const [draftGateway, setDraftGateway] = useState(
    selectedGateway ?? noneGateway,
  );

  useEffect(() => {
    setDraftGateway(selectedGateway ?? noneGateway);
  }, [selectedGateway]);

  function applyUiGateway(next: GatewayConfig) {
    if (draftGateway === noneGateway) {
      delete next.ui;
      return;
    }
    next.ui ??= {};
    if (implicitDefaultUiGateway(next, draftGateway)) {
      delete next.ui.gateways;
    } else {
      next.ui.gateways = draftGateway;
    }
  }

  return (
    <Panel>
      <div className="form-grid">
        <FieldGroup
          label="Public UI gateway"
          tooltip="Which traffic gateway exposes the UI."
        >
          <Dropdown
            ariaLabel="Public UI gateway"
            value={draftGateway}
            options={[
              {
                value: noneGateway,
                label: "None (admin interface only)",
                description: "Do not expose the UI on a traffic gateway.",
              },
              ...gatewayOptions,
            ]}
            disabled={update.isPending}
            onChange={setDraftGateway}
          />
        </FieldGroup>
      </div>
      <div className="button-row">
        <ConfigDiffSaveActions
          config={config.data}
          diffTitle="UI gateway config diff"
          saveLabel="Save UI gateway"
          saving={update.isPending}
          saveDisabled={
            !config.data || draftGateway === (selectedGateway ?? noneGateway)
          }
          onSave={() =>
            update.mutate((next) => {
              applyUiGateway(next);
            })
          }
          applyDiff={applyUiGateway}
        />
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

function uiGateway(config: GatewayConfig | null | undefined) {
  return (
    firstGatewayRef(config?.ui?.gateways) ?? implicitDefaultUiGatewayRef(config)
  );
}

function implicitDefaultUiGatewayRef(config: GatewayConfig | null | undefined) {
  return config?.ui && config.gateways?.default ? "default" : undefined;
}

function implicitDefaultUiGateway(config: GatewayConfig, gateway: string) {
  return Boolean(config.gateways?.default) && gateway === "default";
}
