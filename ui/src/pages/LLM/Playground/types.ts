export interface PlaygroundModel {
  /** Display label (the llm.models[].name pattern, e.g. "*") */
  label: string;
  /** Default model name to send in the request (params.model or empty) */
  defaultModel: string;
  /** Provider label (openAI, gemini, etc.) */
  provider: string;
  /** The base URL to send chat completions to (the gateway endpoint) */
  baseUrl: string;
}

export interface Message {
  role: "user" | "assistant";
  content: string;
}
