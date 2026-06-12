import Form from "@rjsf/antd";
import type { RJSFSchema } from "@rjsf/utils";
import { Alert, Spin } from "antd";
import { useEffect, useState } from "react";
import { validator } from "../../utils/validator";
import { assetUrl } from "../../utils/assetUrl";
import {
  ArrayFieldTemplate,
  CollapsibleObjectFieldTemplate,
  FieldTemplate,
  WrapIfAdditionalTemplate,
} from "../FormTemplates";

interface SchemaFormProps {
  category: "policies" | "listeners" | "routes" | "backends";
  schemaType: string;
  initialData?: any;
  onSubmit: (data: any) => void;
  onChange?: (data: any) => void;
}

/**
 * SchemaForm wrapper component that loads schemas dynamically and renders
 * forms with custom Ant Design templates.
 */
export function SchemaForm({
  category,
  schemaType,
  initialData,
  onSubmit,
  onChange,
}: SchemaFormProps) {
  const [schema, setSchema] = useState<RJSFSchema | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Load schema from generated files
    const loadSchema = async () => {
      setLoading(true);
      setError(null);

      try {
        const schemaPath = assetUrl(`/schema-forms/${category}/${schemaType}.json`);
        const response = await fetch(schemaPath);

        if (!response.ok) {
          throw new Error(`Failed to load schema: ${response.statusText}`);
        }

        const schemaData = await response.json();
        setSchema(schemaData);
      } catch (err) {
        console.error("Error loading schema:", err);
        setError(err instanceof Error ? err.message : "Unknown error");
      } finally {
        setLoading(false);
      }
    };

    loadSchema();
  }, [category, schemaType]);

  if (loading) {
    return (
      <div style={{ textAlign: "center", padding: "40px" }}>
        <Spin size="large" />
      </div>
    );
  }

  if (error || !schema) {
    return (
      <Alert
        type="error"
        message="Failed to load form"
        description={error || "Schema not found"}
        showIcon
      />
    );
  }

  return (
    <Form
      schema={schema}
      validator={validator}
      formData={initialData}
      onSubmit={({ formData }) => onSubmit(formData)}
      onChange={({ formData }) => onChange?.(formData)}
      onError={(errors) => {
        console.error("Form validation errors:", errors);
      }}
      templates={{
        ObjectFieldTemplate: CollapsibleObjectFieldTemplate,
        FieldTemplate: FieldTemplate,
        ArrayFieldTemplate: ArrayFieldTemplate,
        WrapIfAdditionalTemplate,
      }}
      // Ant Design form configuration
      showErrorList={false}
    />
  );
}
