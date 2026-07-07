import { Save, Server } from "lucide-react";
import { useState } from "react";
import {
  GatewayBindingEditor,
  type GatewayBindingValue,
} from "../../components/GatewayBindingEditor";
import { Drawer, StatusBanner } from "../../components/Primitives";
import { PolicySection } from "../../policies/PolicyLayout";
import type { SchemaHelp } from "../../schemaHelp";
import type { GatewayConfig, LlmConfig } from "../../types";

export type LlmSettingsPatch = Partial<Omit<LlmConfig, "gateways" | "port">> & {
  gateways?: LlmConfig["gateways"] | null;
  port?: number | null;
};

export function LlmSettingsDrawer(props: {
  config?: GatewayConfig | null;
  llm?: LlmConfig | null;
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onClose: () => void;
  onSave: (settings: LlmSettingsPatch) => void;
}) {
  const [binding, setBinding] = useState<GatewayBindingValue>({
    gateways: props.llm?.gateways ?? null,
    port: props.llm?.port ?? null,
  });

  return (
    <Drawer title="Settings" onClose={props.onClose}>
      <form
        className="policy-editor-stack"
        onSubmit={(event) => {
          event.preventDefault();
          props.onSave({
            gateways: binding.gateways ?? null,
            port: binding.gateways ? null : (binding.port ?? null),
          });
        }}
      >
        <PolicySection
          icon={<Server size={17} />}
          title="Gateway binding"
          description="Choose how LLM traffic is exposed."
        >
          <div className="form-grid">
            <GatewayBindingEditor
              config={props.config}
              value={binding}
              defaultPort={4000}
              portLabel="Port"
              portPlaceholder="4000"
              portTooltip={props.help.field<LlmConfig>(
                "LocalLLMConfig",
                "port",
                "Gateway port for LLM traffic.",
              )}
              onChange={setBinding}
            />
          </div>
        </PolicySection>
        <button className="button" type="submit" disabled={props.saving}>
          <Save size={16} />
          Save settings
        </button>
      </form>
      {props.saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {props.saveError}
        </StatusBanner>
      ) : null}
    </Drawer>
  );
}
