import { tr } from "../i18n";
import { useEffect, useMemo, useState } from "react";
import {
  Dropdown,
  FieldGroup,
  Panel,
  StatusBanner,
} from "../components/Primitives";
import { ConfigDiffSaveActions } from "../components/ConfigDiffDrawer";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import type { LocalUIConfig } from "../gateway-config";
import type { GatewayConfig, TrafficGateway } from "../types";
import type { PolicyKey } from "../policies/types";
import { PolicyCatalogPage } from "./Policies";

const noneGateway = "__none__";

const uiPolicySections: Array<{ title: string; keys: PolicyKey[] }> = [
  {
    get title() {
      return tr("copy.uiAccessPolicies");
    },
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
      title={tr("copy.uiSettings")}
      description={tr(
        "copy.exposeTheUiOnATrafficGatewayAndConfigurePoliciesThatProtectTheUi",
      )}
      schemaRoot="LocalUIPolicy"
      sections={uiPolicySections}
      yamlDescription="Read-only view of UI policies from ui.policies."
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
  const config = useGatewayConfig();
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
          label={tr("copy.publicUiGateway")}
          tooltip={tr("copy.whichTrafficGatewayExposesTheUi")}
        >
          <Dropdown
            ariaLabel="Public UI gateway"
            value={draftGateway}
            options={[
              {
                value: noneGateway,
                label: tr("copy.noneAdminInterfaceOnly"),
                description: tr("copy.doNotExposeTheUiOnATrafficGateway"),
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
          saveLabel={tr("copy.saveUiGateway")}
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
        <StatusBanner state="warn" title={tr("copy.noGatewaysConfigured")}>
          {tr("copy.addAGatewayBeforeExposingTheUi")}
        </StatusBanner>
      ) : null}
      {update.isError ? (
        <StatusBanner state="bad" title={tr("copy.saveFailed")}>
          {update.error.message}
        </StatusBanner>
      ) : null}
      {update.isSuccess ? (
        <StatusBanner state="ok" title={tr("copy.gatewaySaved")} />
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
        label: tr("copy.valueAllListeners", [name]),
        description: gatewayDescription(gateway),
      },
      ...listeners.map((listener, index) => {
        const listenerName = listener.name ?? `listener${index}`;
        return {
          value: `${name}/${listenerName}`,
          label: tr("copy.valueValue", [name, listenerName]),
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
