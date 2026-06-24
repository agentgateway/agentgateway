import type { ProviderName } from "../types";
import { providerDisplayName } from "../config";
import agwIcon from "../assets/agw-icon.svg";
import anthropicIcon from "../assets/providers/anthropic.svg";
import azureIcon from "../assets/providers/azure.svg";
import basetenIcon from "../assets/providers/baseten.svg";
import bedrockIcon from "../assets/providers/bedrock.svg";
import cerebrasIcon from "../assets/providers/cerebras.svg";
import cohereIcon from "../assets/providers/cohere.svg";
import copilotIcon from "../assets/providers/copilot.svg";
import deepinfraIcon from "../assets/providers/deepinfra.svg";
import deepseekIcon from "../assets/providers/deepseek.svg";
import fireworksIcon from "../assets/providers/fireworks.svg";
import geminiIcon from "../assets/providers/gemini.svg";
import groqIcon from "../assets/providers/groq.svg";
import huggingfaceIcon from "../assets/providers/huggingface.svg";
import mistralIcon from "../assets/providers/mistral.svg";
import ollamaIcon from "../assets/providers/ollama.svg";
import openAiIcon from "../assets/providers/openai.svg";
import openrouterIcon from "../assets/providers/openrouter.svg";
import togetheraiIcon from "../assets/providers/togetherai.svg";
import vertexIcon from "../assets/providers/vertex.svg";
import xaiIcon from "../assets/providers/xai.svg";

const providerIcons: Record<string, string> = {
  openai: openAiIcon,
  openAI: openAiIcon,
  anthropic: anthropicIcon,
  gemini: geminiIcon,
  vertex: vertexIcon,
  bedrock: bedrockIcon,
  azure: azureIcon,
  copilot: copilotIcon,
  cohere: cohereIcon,
  ollama: ollamaIcon,
  baseten: basetenIcon,
  cerebras: cerebrasIcon,
  deepinfra: deepinfraIcon,
  deepseek: deepseekIcon,
  groq: groqIcon,
  huggingface: huggingfaceIcon,
  mistral: mistralIcon,
  openrouter: openrouterIcon,
  togetherai: togetheraiIcon,
  xai: xaiIcon,
  fireworks: fireworksIcon,
  custom: agwIcon,
};

const monochromeProviders = new Set<string>([
  "openai",
  "openAI",
  "anthropic",
  "baseten",
  "copilot",
  "ollama",
  "openrouter",
  "togetherai",
  "xai",
]);

export function ProviderIcon(props: { provider: ProviderName | string }) {
  const icon = providerIcons[props.provider];
  if (!icon)
    return (
      <span className="provider-icon provider-icon-fallback">
        {providerInitials(props.provider)}
      </span>
    );
  return (
    <img
      className={
        monochromeProviders.has(props.provider)
          ? "provider-icon provider-icon-mono"
          : "provider-icon"
      }
      src={icon}
      alt=""
      aria-hidden="true"
    />
  );
}

function providerInitials(provider: ProviderName | string) {
  const display = providerDisplayName(provider);
  const words = display.split(/\s+/).filter(Boolean);
  if (display === "xAI") return "xA";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return words
    .slice(0, 2)
    .map((word) => word[0])
    .join("")
    .toUpperCase();
}
