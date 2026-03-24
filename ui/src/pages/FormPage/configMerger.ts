import Ajv from "ajv";
import type { LocalConfig } from "../../api/types";
import { assetUrl } from "../../utils/assetUrl";

type Category = "policies" | "listeners" | "routes" | "backends";

function allowAdditionalProperties(obj: any): void {
  if (typeof obj === "object" && obj !== null) {
    if (obj.additionalProperties === false) {
      obj.additionalProperties = true;
    }
    for (const key in obj) {
      allowAdditionalProperties(obj[key]);
    }
  }
}

const ajv = new Ajv({ allErrors: true, validateSchema: false, strict: false });

/**
 * Validates data against the JSON schema for the given category and type.
 */
async function validateAgainstSchema(
  category: Category,
  schemaType: string,
  data: any,
): Promise<void> {
  const schemaUrl = assetUrl(`/schema-forms/${category}/${schemaType}.json`);
  const response = await fetch(schemaUrl);
  if (!response.ok) {
    throw new Error(`Failed to load schema for ${category}/${schemaType}`);
  }
  const schema = await response.json();
  allowAdditionalProperties(schema);

  const validate = ajv.compile(schema);
  const valid = validate(data);
  if (!valid) {
    const errors =
      validate.errors
        ?.map((err: any) => `${err.instancePath} ${err.message}`)
        .join(", ") || "Unknown validation error";
    throw new Error(`Validation failed: ${errors}`);
  }
}

/**
 * Merges form data into the config based on the category.
 * Handles both simple objects and nested config structures.
 */
export async function mergeFormDataIntoConfig(
  config: LocalConfig,
  category: Category,
  data: any,
): Promise<LocalConfig> {
  const newConfig = { ...config };

  // If data has the full nested binds structure, extract the relevant parts
  let formBind: any;

  if (data?.binds?.[0]) {
    formBind = data.binds[0];
  } else {
    // For direct form submissions (e.g., listener forms), data is the object itself
    // No additional setup needed
  }

  // Ensure binds array exists
  if (!newConfig.binds || newConfig.binds.length === 0) {
    newConfig.binds = [
      {
        port: formBind?.port || 8080,
        listeners: [],
      },
    ];
  }

  // Handle simple (non-nested) data structures
  switch (category) {
    case "listeners": {
      let listenerData = data;
      if (data.binds?.[0]?.listeners?.[0]) {
        listenerData = data.binds[0].listeners[0];
      } else if (data.listener) {
        listenerData = data.listener;
      }

      // Validate against schema
      await validateAgainstSchema(category, "LocalListener", listenerData);

      // Add listener to the first bind, or create a bind if none exists
      if (!newConfig.binds || newConfig.binds.length === 0) {
        newConfig.binds = [
          { port: formBind?.port || 8080, listeners: [listenerData] },
        ];
      } else {
        newConfig.binds[0].listeners = newConfig.binds[0].listeners || [];
        newConfig.binds[0].listeners.push(listenerData);
      }
      break;
    }
    case "routes": {
      // Add route to the first listener of the first bind
      if (!newConfig.binds?.[0]?.listeners?.[0]) {
        throw new Error("No listener found. Please create a listener first.");
      }
      newConfig.binds[0].listeners[0].routes =
        newConfig.binds[0].listeners[0].routes || [];
      newConfig.binds[0].listeners[0].routes.push(data);
      break;
    }
    case "backends": {
      // Add backend to the first route of the first listener
      if (!newConfig.binds?.[0]?.listeners?.[0]?.routes?.[0]) {
        throw new Error("No route found. Please create a route first.");
      }
      newConfig.binds[0].listeners[0].routes[0].backends =
        newConfig.binds[0].listeners[0].routes[0].backends || [];
      newConfig.binds[0].listeners[0].routes[0].backends.push(data);
      break;
    }
    case "policies": {
      // Merge policies into the first route of the first listener
      if (!newConfig.binds?.[0]?.listeners?.[0]?.routes?.[0]) {
        throw new Error("No route found. Please create a route first.");
      }
      newConfig.binds[0].listeners[0].routes[0].policies = {
        ...newConfig.binds[0].listeners[0].routes[0].policies,
        ...data,
      };
      break;
    }
  }

  return newConfig;
}
