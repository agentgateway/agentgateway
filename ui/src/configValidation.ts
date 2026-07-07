import Ajv2020 from "ajv/dist/2020";
import type { ErrorObject } from "ajv";
import configSchema from "../../schema/config.json";
import type { GatewayConfig } from "./types";

let validatorPromise: Promise<ReturnType<Ajv2020["compile"]>> | null = null;

export async function validateGatewayConfig(config: GatewayConfig) {
  const errors = await getGatewayConfigValidationErrors(config);
  if (errors.length === 0) return;

  const messages = errors.slice(0, 5).map((error) => {
    const path = error.instancePath || "/";
    return `${path}: ${error.message ?? "invalid value"}`;
  });
  throw new Error(`Configuration validation failed: ${messages.join("; ")}`);
}

export async function getGatewayConfigValidationErrors(
  config: GatewayConfig,
): Promise<ErrorObject[]> {
  const validate = await getValidator();
  if (validate(config)) return [];
  return validate.errors?.slice() ?? [];
}

async function getValidator() {
  validatorPromise ??= Promise.resolve().then(() => {
    const schema = structuredClone(configSchema);
    sanitizeEmptyEnums(schema);
    const ajv = new Ajv2020({
      allErrors: true,
      validateFormats: false,
      strict: false,
    });
    return ajv.compile(schema);
  });
  return validatorPromise;
}

function sanitizeEmptyEnums(value: unknown): unknown {
  if (!value || typeof value !== "object") return value;
  if (Array.isArray(value)) {
    value.forEach(sanitizeEmptyEnums);
    return value;
  }

  const object = value as Record<string, unknown>;
  if (Array.isArray(object.enum) && object.enum.length === 0) {
    delete object.enum;
    delete object.type;
    object.not = {};
  }
  Object.values(object).forEach(sanitizeEmptyEnums);
  return object;
}
