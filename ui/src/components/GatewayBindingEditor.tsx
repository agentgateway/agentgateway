import { useEffect } from "react";
import { Dropdown, Field, FieldGroup } from "./Primitives";
import type { GatewayConfig } from "../types";

const dedicatedPort = "__port__";

export type GatewayBindingValue = {
  gateways?: string | string[] | null;
  port?: number | null;
};

export function GatewayBindingEditor(props: {
  config: GatewayConfig | null | undefined;
  value: GatewayBindingValue;
  defaultPort: number;
  portLabel?: string;
  gatewayLabel?: string;
  portTooltip?: string;
  portPlaceholder?: string;
  disabled?: boolean;
  onChange: (value: GatewayBindingValue) => void;
}) {
  const options = gatewayOptions(props.config);
  const gateway = firstGatewayRef(props.value.gateways);
  const hasDedicatedPort =
    props.value.port !== null && props.value.port !== undefined;
  const showDedicatedPort = !gateway && hasDedicatedPort;
  const selectedValue = gateway ?? (showDedicatedPort ? dedicatedPort : "");

  useEffect(() => {
    if (!gateway && !hasDedicatedPort && options[0]) {
      props.onChange({ gateways: options[0].value, port: null });
    }
  }, [gateway, hasDedicatedPort, options, props]);

  if (!options.length) {
    return (
      <PortField
        label={props.portLabel ?? "Port"}
        tooltip={props.portTooltip}
        value={props.value.port}
        placeholder={props.portPlaceholder ?? String(props.defaultPort)}
        disabled={props.disabled}
        onChange={(port) => props.onChange({ port, gateways: null })}
      />
    );
  }

  return (
    <>
      <FieldGroup label={props.gatewayLabel ?? "Gateway"}>
        <Dropdown
          ariaLabel={props.gatewayLabel ?? "Gateway"}
          value={selectedValue}
          options={
            showDedicatedPort
              ? [
                  {
                    value: dedicatedPort,
                    label: "Dedicated port",
                    description: "Bind this surface on its own listener port.",
                  },
                  ...options,
                ]
              : options
          }
          disabled={props.disabled}
          onChange={(value) => {
            if (value === dedicatedPort) {
              props.onChange({
                gateways: null,
                port: props.value.port ?? props.defaultPort,
              });
            } else {
              props.onChange({ gateways: value, port: null });
            }
          }}
        />
      </FieldGroup>
      {showDedicatedPort ? (
        <PortField
          label={props.portLabel ?? "Port"}
          tooltip={props.portTooltip}
          value={props.value.port}
          placeholder={props.portPlaceholder ?? String(props.defaultPort)}
          disabled={props.disabled}
          onChange={(port) => props.onChange({ port, gateways: null })}
        />
      ) : null}
    </>
  );
}

function PortField(props: {
  label: string;
  tooltip?: string;
  value?: number | null;
  placeholder: string;
  disabled?: boolean;
  onChange: (port: number | null) => void;
}) {
  return (
    <Field label={props.label} tooltip={props.tooltip}>
      <input
        value={props.value?.toString() ?? ""}
        disabled={props.disabled}
        onChange={(event) => {
          const parsed = Number(event.target.value);
          props.onChange(
            event.target.value.trim() && Number.isFinite(parsed)
              ? parsed
              : null,
          );
        }}
        placeholder={props.placeholder}
      />
    </Field>
  );
}

function gatewayOptions(config: GatewayConfig | null | undefined) {
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
        description: `${listeners.length} listeners`,
      },
      ...listeners.map((listener, index) => {
        const listenerName = listener.name ?? `listener${index}`;
        return {
          value: `${name}/${listenerName}`,
          label: `${name}/${listenerName}`,
          description: gateway.port ? `Port ${gateway.port}` : undefined,
        };
      }),
    ];
  });
}

function firstGatewayRef(gateways: string | string[] | null | undefined) {
  if (Array.isArray(gateways)) return gateways[0];
  return gateways ?? undefined;
}
