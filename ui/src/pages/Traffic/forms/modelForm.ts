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
      description: "Modify headers sent to the provider",
      additionalProperties: true,
      properties: {
        add: {
          type: "object",
          title: "Add Headers",
          description: "Headers to add to requests",
          additionalProperties: {
            type: "string",
          },
        },
        set: {
          type: "object",
          title: "Set Headers",
          description: "Headers to set/override in requests",
          additionalProperties: {
            type: "string",
          },
        },
        remove: {
          type: "array",
          title: "Remove Headers",
          description: "Header names to remove from requests",
          items: {
            type: "string",
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
    "ui:title": "",
    "ui:help": "Example: {\"model\": \"request.model + '-latest'\"}",
  },
  requestHeaders: {
    "ui:title": "",
    "ui:help": "Modify headers sent to the LLM provider",
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
