import type { Dispatch, SetStateAction } from "react";
import type { SchemaHelp } from "../../schemaHelp";
import type {
  CustomProvider,
  LlmModel,
  ModelProvider,
  ProviderFormat,
} from "../../types";
import type { ProviderFormatConfig } from "../../gateway-config";

const formats: ProviderFormat[] = [
  "completions",
  "messages",
  "responses",
  "embeddings",
  "anthropicTokenCount",
  "realtime",
  "rerank",
];

const formatLabels: Record<ProviderFormat, string> = {
  completions: "Chat completions (/v1/chat/completions)",
  messages: "Anthropic messages (/v1/messages)",
  responses: "Responses (/v1/responses)",
  embeddings: "Embeddings (/v1/embeddings)",
  anthropicTokenCount: "Anthropic token count (/v1/messages/count_tokens)",
  realtime: "Realtime (/v1/realtime)",
  rerank: "Rerank (/v2/rerank)",
};

export function CustomFormats(props: {
  model: LlmModel;
  help: SchemaHelp;
  setModel: Dispatch<SetStateAction<LlmModel>>;
}) {
  const custom = customProvider(props.model.provider);

  function toggle(type: ProviderFormat, checked: boolean) {
    props.setModel((current) => {
      const currentCustom = customProvider(current.provider);
      const nextFormats = checked
        ? [...currentCustom.formats, { type }]
        : currentCustom.formats.filter(
            (format: ProviderFormatConfig) => format.type !== type,
          );
      return {
        ...current,
        provider: { custom: { ...currentCustom, formats: nextFormats } },
      };
    });
  }

  function setPath(type: ProviderFormat, path: string) {
    props.setModel((current) => {
      if (
        typeof current.provider === "string" ||
        !("custom" in current.provider)
      )
        return current;
      return {
        ...current,
        provider: {
          custom: {
            ...current.provider.custom,
            formats: current.provider.custom.formats.map(
              (format: ProviderFormatConfig) =>
                format.type === type
                  ? { ...format, path: path || null }
                  : format,
            ),
          },
        },
      };
    });
  }

  return (
    <div className="format-grid">
      {formats.map((type) => {
        const selected = custom.formats.find(
          (format: ProviderFormatConfig) => format.type === type,
        );
        return (
          <div className="format-row" key={type}>
            <label
              className={selected ? "format-toggle selected" : "format-toggle"}
            >
              <input
                type="checkbox"
                checked={Boolean(selected)}
                onChange={(event) => toggle(type, event.target.checked)}
              />
              <span className="format-toggle-box" aria-hidden="true" />
              <span>{formatLabels[type]}</span>
            </label>
            <input
              aria-label={`${formatLabels[type]} path override`}
              disabled={!selected}
              value={selected?.path ?? ""}
              placeholder={props.help.field<ProviderFormatConfig>(
                "ProviderFormatConfig",
                "path",
                "optional path override",
              )}
              onChange={(event) => setPath(type, event.target.value)}
            />
          </div>
        );
      })}
    </div>
  );
}

function customProvider(provider: ModelProvider): CustomProvider {
  if (typeof provider === "object" && "custom" in provider)
    return provider.custom;
  return { formats: [] };
}
