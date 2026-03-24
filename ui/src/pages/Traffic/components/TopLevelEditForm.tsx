import { Button, Space } from "antd";
import { useState } from "react";
import toast from "react-hot-toast";
import Form from "@rjsf/antd";
import validator from "@rjsf/validator-ajv8";
import { forms } from "../forms";
import * as api from "../../../api/crud";
import { stripFormDefaults } from "../../../api/helpers";
import {
  ArrayFieldTemplate,
  CollapsibleObjectFieldTemplate,
  FieldTemplate,
  WrapIfAdditionalTemplate,
} from "../../../components/FormTemplates";

export interface TopLevelEditTarget {
  type: "llm" | "mcp" | "frontendPolicies" | "backend" | "policy";
  initialData?: Record<string, unknown>;
}

interface TopLevelEditFormProps {
  target: TopLevelEditTarget;
  onSaved: () => void;
  onCancel: () => void;
}

export function TopLevelEditForm({
  target,
  onSaved,
  onCancel,
}: TopLevelEditFormProps) {
  const [formData, setFormData] = useState<any>(
    target.initialData || getDefaultValues(target.type),
  );
  const [isSaving, setIsSaving] = useState(false);

  const form = forms[target.type];
  if (!form) {
    return <div>Form not found for type: {target.type}</div>;
  }

  const handleError = (errors: any) => {
    // Show first validation error in toast
    if (errors && errors.length > 0) {
      const firstError = errors[0];
      const errorMessage = firstError.stack || firstError.message || "Validation error";
      toast.error(errorMessage);
    }
  };

  const handleSubmit = async ({ formData: rawData }: any) => {
    setIsSaving(true);
    try {
      // Apply form-specific transformation if available
      let transformedData = rawData;
      if (form.transformBeforeSubmit) {
        transformedData = form.transformBeforeSubmit(rawData);
      }

      // Determine which keys to preserve at top level for oneOf fields
      const topLevelKeysToKeep = getTopLevelKeysToPreserve(target.type, transformedData);

      // Strip form defaults (null values, empty arrays) except for specified keys
      const cleanedData = (stripFormDefaults(transformedData, topLevelKeysToKeep) || {}) as Record<string, unknown>;

      // Call appropriate API function based on type
      switch (target.type) {
        case "llm":
          await api.createOrUpdateLLM(cleanedData);
          toast.success("LLM configuration saved successfully");
          break;
        case "mcp":
          await api.createOrUpdateMCP(cleanedData);
          toast.success("MCP configuration saved successfully");
          break;
        case "frontendPolicies":
          await api.createOrUpdateFrontendPolicies(cleanedData);
          toast.success("Frontend policies saved successfully");
          break;
        case "backend":
          await api.createTopLevelBackend(cleanedData);
          toast.success("Backend created successfully");
          break;
        case "policy":
          await api.createPolicy(cleanedData);
          toast.success("Policy created successfully");
          break;
        default:
          throw new Error(`Unknown type: ${target.type}`);
      }

      onSaved();
    } catch (error: unknown) {
      console.error("Failed to save:", error);
      const errorMessage =
        error && typeof error === "object" && "message" in error && typeof error.message === "string"
          ? error.message
          : "Failed to save configuration";
      toast.error(errorMessage);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Form
      schema={form.schema}
      uiSchema={form.uiSchema}
      formData={formData}
      validator={validator}
      onChange={(e) => setFormData(e.formData)}
      onSubmit={handleSubmit}
      onError={handleError}
      disabled={isSaving}
      templates={{
        ObjectFieldTemplate: CollapsibleObjectFieldTemplate,
        FieldTemplate,
        ArrayFieldTemplate,
        WrapIfAdditionalTemplate,
      }}
    >
      <Space style={{ marginTop: 24 }}>
        <Button type="primary" htmlType="submit" loading={isSaving}>
          Save
        </Button>
        <Button onClick={onCancel} disabled={isSaving}>
          Cancel
        </Button>
      </Space>
    </Form>
  );
}

/**
 * Get default values for a given resource type
 */
function getDefaultValues(type: string): any {
  const form = forms[type as keyof typeof forms];
  return form?.defaultValues || {};
}

/**
 * Determine which top-level keys should preserve empty arrays.
 * This is important for oneOf fields where the presence of an empty
 * array indicates which variant is active.
 */
function getTopLevelKeysToPreserve(
  type: string,
  data: Record<string, unknown>,
): ReadonlySet<string> | undefined {
  // For MCP config, preserve the active target type field
  if (type === "mcp" && data.targets && Array.isArray(data.targets)) {
    const preserveKeys = new Set<string>();
    data.targets.forEach((target: any) => {
      if (target.sse !== undefined) preserveKeys.add("sse");
      if (target.mcp !== undefined) preserveKeys.add("mcp");
      if (target.stdio !== undefined) preserveKeys.add("stdio");
      if (target.openapi !== undefined) preserveKeys.add("openapi");
    });
    return preserveKeys;
  }

  // For policy config, preserve the active target type
  if (type === "policy" && data.target) {
    const target = data.target as any;
    const preserveKeys = new Set<string>();
    if (target.gateway !== undefined) preserveKeys.add("gateway");
    if (target.route !== undefined) preserveKeys.add("route");
    if (target.backend !== undefined) preserveKeys.add("backend");
    return preserveKeys;
  }

  return undefined;
}
