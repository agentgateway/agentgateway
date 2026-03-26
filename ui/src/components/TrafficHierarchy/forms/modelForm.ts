import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import type { LocalLLMModels } from "../../../config";

/**
 * Manually configured JSON Schema for LLM Model
 * Handcrafted to match LocalLLMModels type from config.d.ts
 */
export const schema: RJSFSchema = {
  type: "object",
  required: ["name", "provider"],
  additionalProperties: true,
  properties: {
    name: {
      type: "string",
      title: "Model Name",
      description: "Name of the model to match from user requests",
    },
    provider: {
      type: "string",
      title: "Provider",
      enum: ["openAI", "gemini", "vertex", "anthropic", "bedrock", "azureOpenAI"],
      description: "LLM provider to connect to",
    },
    params: {
      type: "object",
      description: "Model-specific parameters (API keys, regions, etc.)",
      additionalProperties: true,
      properties: {
        model: {
          type: "string",
          title: "Model Name",
          description: "Model name to use with the provider (e.g., gpt-4-turbo)",
        },
        apiKey: {
          type: "string",
          title: "API Key",
          description: "API key for authentication (consider using environment variables)",
        },
        awsRegion: {
          type: "string",
          title: "AWS Region",
          description: "AWS region for Bedrock (e.g., us-east-1)",
        },
        vertexRegion: {
          type: "string",
          title: "Vertex Region",
          description: "Google Cloud region for Vertex AI (e.g., us-central1)",
        },
        vertexProject: {
          type: "string",
          title: "Vertex Project",
          description: "Google Cloud project ID for Vertex AI",
        },
        azureHost: {
          type: "string",
          title: "Azure Host",
          description: "Azure OpenAI deployment host",
        },
        azureApiVersion: {
          type: "string",
          title: "Azure API Version",
          description: "Azure OpenAI API version (e.g., 2024-02-01)",
        },
      },
    },
    defaults: {
      type: "object",
      description: "Default values applied when fields are missing from requests",
      additionalProperties: true,
    },
    overrides: {
      type: "object",
      description: "Values that override user-provided fields in requests",
      additionalProperties: true,
    },
    transformation: {
      type: "object",
      description: "CEL expressions to transform request fields",
      additionalProperties: {
        type: "string",
      },
    },
    requestHeaders: {
      type: "object",
      title: "Request Headers",
      description: "Modify headers sent to the provider",
      properties: {
        add: {
          type: "object",
          title: "Add Headers",
          description: "Headers to add to requests",
          additionalProperties: { type: "string" },
        },
        set: {
          type: "object",
          title: "Set Headers",
          description: "Headers to set/override in requests",
          additionalProperties: { type: "string" },
        },
        remove: {
          type: "array",
          title: "Remove Headers",
          description: "Header names to remove from requests",
          items: { type: "string" },
        },
      },
    },
    responseHeaders: {
      type: "object",
      title: "Response Headers",
      description: "Modify headers in responses from the provider",
      properties: {
        add: {
          type: "object",
          title: "Add Headers",
          description: "Headers to add to responses",
          additionalProperties: { type: "string" },
        },
        set: {
          type: "object",
          title: "Set Headers",
          description: "Headers to set/override in responses",
          additionalProperties: { type: "string" },
        },
        remove: {
          type: "array",
          title: "Remove Headers",
          description: "Header names to remove from responses",
          items: { type: "string" },
        },
      },
    },
    backendTLS: {
      type: "object",
      title: "Backend TLS",
      description: "TLS configuration when connecting to the LLM provider",
      properties: {
        cert: { type: "string", title: "Certificate" },
        key: { type: "string", title: "Key" },
        root: { type: "string", title: "Root CA" },
        hostname: { type: "string", title: "Hostname", description: "SNI hostname override" },
        insecure: { type: "boolean", title: "Insecure", description: "Skip TLS verification" },
        insecureHost: { type: "boolean", title: "Insecure Host", description: "Skip hostname verification" },
        alpn: { type: "array", title: "ALPN Protocols", items: { type: "string" } },
        subjectAltNames: { type: "array", title: "Subject Alt Names", items: { type: "string" } },
      },
    },
    health: {
      type: "object",
      title: "Health Policy",
      description: "Outlier detection for this model backend",
      properties: {
        unhealthyExpression: {
          type: "string",
          title: "Unhealthy Expression",
          description: "CEL expression; true means unhealthy (e.g., response.code >= 500)",
        },
        eviction: {
          type: "object",
          title: "Eviction",
          properties: {
            duration: { type: "string", title: "Duration", description: "How long to evict (e.g., 30s)" },
            restoreHealth: { type: "number", title: "Restore Health", description: "Number of successes to restore" },
            consecutiveFailures: { type: "number", title: "Consecutive Failures", description: "Failures before eviction" },
            healthThreshold: { type: "number", title: "Health Threshold" },
          },
        },
      },
    },
    backendTunnel: {
      type: "object",
      title: "Backend Tunnel",
      description: "Tunnel configuration when connecting to the LLM provider",
      properties: {
        proxy: {
          type: "object",
          description: "Proxy address",
          properties: {
            host: { type: "string", title: "Host", description: "Proxy hostname or IP" },
          },
        },
      },
    },
    guardrails: {
      type: "object",
      description: "Content safety and validation rules",
      additionalProperties: true,
      properties: {
        request: {
          type: "array",
          title: "Request Guards",
          description: "Guardrails applied to incoming requests",
          items: {
            type: "object",
            additionalProperties: true,
          },
        },
        response: {
          type: "array",
          title: "Response Guards",
          description: "Guardrails applied to provider responses",
          items: {
            type: "object",
            additionalProperties: true,
          },
        },
      },
    },
    matches: {
      type: "array",
      title: "Route Matches",
      description: "Conditions for selecting this model (e.g., based on headers)",
      items: {
        type: "object",
        additionalProperties: true,
        properties: {
          headers: {
            type: "array",
            title: "Header Matches",
            items: {
              type: "object",
              properties: {
                name: {
                  type: "string",
                  title: "Header Name",
                },
                value: {
                  type: "string",
                  title: "Header Value",
                  description: "Value to match (can be exact, regex, etc.)",
                },
              },
            },
          },
        },
      },
    },
  },
};

/**
 * UI Schema for LLM Model
 */
export const uiSchema: UiSchema = {
  "ui:title": "",
  name: {
    "ui:placeholder": "gpt-4",
    "ui:help": "Model name that users will request",
  },
  provider: {
    "ui:widget": "select",
  },
  params: {
    "ui:title": "",
    model: {
      "ui:placeholder": "gpt-4-turbo",
      "ui:help": "Override the model name sent to the provider",
    },
    apiKey: {
      "ui:placeholder": "sk-...",
      "ui:help": "Prefer using environment variables for sensitive keys",
      "ui:widget": "password",
    },
    awsRegion: {
      "ui:placeholder": "us-east-1",
      "ui:help": "Required for AWS Bedrock provider",
    },
    vertexRegion: {
      "ui:placeholder": "us-central1",
      "ui:help": "Required for Google Vertex AI provider",
    },
    vertexProject: {
      "ui:placeholder": "my-gcp-project",
      "ui:help": "Required for Google Vertex AI provider",
    },
    azureHost: {
      "ui:placeholder": "my-deployment.openai.azure.com",
      "ui:help": "Required for Azure OpenAI provider",
    },
    azureApiVersion: {
      "ui:placeholder": "2024-02-01",
      "ui:help": "Required for Azure OpenAI provider",
    },
  },
  defaults: {
    "ui:title": "",
    "ui:help": "Example: {\"temperature\": 0.7, \"max_tokens\": 1000}",
  },
  overrides: {
    "ui:title": "",
    "ui:help": "Example: {\"top_p\": 1.0} - forces this value even if user provides different",
  },
  transformation: {
    "ui:field": "keyValueMap",
    "ui:keyPlaceholder": "field",
    "ui:valuePlaceholder": "CEL expression",
  },
  requestHeaders: {
    "ui:help": "Modify headers sent to the LLM provider",
    add: {
      "ui:field": "keyValueMap",
      "ui:keyPlaceholder": "header-name",
      "ui:valuePlaceholder": "header-value",
    },
    set: {
      "ui:field": "keyValueMap",
      "ui:keyPlaceholder": "header-name",
      "ui:valuePlaceholder": "header-value",
    },
    remove: { "ui:help": "Header names to remove from requests" },
  },
  responseHeaders: {
    "ui:help": "Modify headers in responses from the LLM provider",
    add: {
      "ui:field": "keyValueMap",
      "ui:keyPlaceholder": "header-name",
      "ui:valuePlaceholder": "header-value",
    },
    set: {
      "ui:field": "keyValueMap",
      "ui:keyPlaceholder": "header-name",
      "ui:valuePlaceholder": "header-value",
    },
    remove: { "ui:help": "Header names to remove from responses" },
  },
  backendTLS: {
    cert: { "ui:widget": "textarea", "ui:options": { rows: 3 } },
    key: { "ui:widget": "textarea", "ui:options": { rows: 3 } },
    root: { "ui:widget": "textarea", "ui:options": { rows: 3 } },
  },
  health: {
    unhealthyExpression: {
      "ui:placeholder": "response.code >= 500",
    },
    eviction: {
      duration: { "ui:placeholder": "30s" },
    },
  },
  backendTunnel: {
    proxy: {
      host: { "ui:placeholder": "proxy.example.com" },
    },
  },
  guardrails: {
    "ui:title": "",
    "ui:help": "Advanced: Add content safety filters and validation rules",
  },
  matches: {
    "ui:help": "Advanced: Route to this model based on request headers",
  },
};

/**
 * Default values for a new model
 */
export const defaultValues: Partial<LocalLLMModels> = {
  name: "gpt-4",
  provider: "openAI",
};

/**
 * Type guard to validate data matches LocalLLMModels
 */
export function isLocalLLMModels(data: unknown): data is LocalLLMModels {
  return (
    typeof data === "object" &&
    data !== null &&
    "name" in data &&
    typeof (data as any).name === "string" &&
    "provider" in data &&
    typeof (data as any).provider === "string"
  );
}

/**
 * Transform function - no transformation needed
 */
export function transformBeforeSubmit(data: unknown): unknown {
  return data;
}
