import type { ObjectFieldTemplateProps } from "@rjsf/utils";
import { Typography } from "antd";
import { useMemo } from "react";
import { HideLabelContext } from "./HideLabelContext";

const { Title } = Typography;

// Maximum nesting level before we stop indenting further
const MAX_NESTING_LEVEL = 2;

/**
 * Custom ObjectFieldTemplate that:
 * - Renders required fields at the top, then optional fields — always visible.
 * - Provides HideLabelContext so FieldTemplate can suppress labels that
 *   duplicate the parent section title.
 */
export function CollapsibleObjectFieldTemplate(
  props: ObjectFieldTemplateProps,
) {
  const {
    title,
    description,
    properties,
    required = [],
    uiSchema,
    idSchema,
  } = props;

  // Track nesting level to limit indentation
  const nestingLevel = (uiSchema?.["ui:nestingLevel"] as number) || 0;
  const effectiveNestingLevel = Math.min(nestingLevel, MAX_NESTING_LEVEL);
  const leftPadding = effectiveNestingLevel * 12;

  const sectionTitle = title?.toString() || "";
  const sectionTitleLower = sectionTitle.toLowerCase().trim();

  // Compute the set of field IDs whose labels duplicate the section title.
  const idsToHideLabel = useMemo(() => {
    const ids = new Set<string>();
    if (!sectionTitleLower) return ids;
    for (const prop of properties) {
      if (prop.name.toLowerCase().trim() === sectionTitleLower) {
        const s = (idSchema as Record<string, { $id: string }>)[prop.name];
        if (s?.$id) ids.add(s.$id);
      }
    }
    return ids;
  }, [properties, sectionTitleLower, idSchema]);

  // Categorize fields: required first, then optional — never collapsed.
  const { requiredFields, optionalFields } = useMemo(() => {
    const req: typeof properties = [];
    const opt: typeof properties = [];
    const requiredArray: string[] = Array.isArray(required) ? required : [];
    for (const prop of properties) {
      if (requiredArray.includes(prop.name)) {
        req.push(prop);
      } else {
        opt.push(prop);
      }
    }
    return { requiredFields: req, optionalFields: opt };
  }, [properties, required]);

  return (
    <HideLabelContext.Provider value={idsToHideLabel}>
      <div
        className="object-field-template"
        style={{ paddingLeft: `${leftPadding}px` }}
      >
        {title && (
          <Title level={5} style={{ marginBottom: 12, marginTop: 4 }}>
            {title}
          </Title>
        )}
        {description && (
          <Typography.Paragraph type="secondary" style={{ marginBottom: 12 }}>
            {description}
          </Typography.Paragraph>
        )}

        {requiredFields.map((prop) => (
          <div key={prop.name}>{prop.content}</div>
        ))}

        {optionalFields.length > 0 && (
          <div style={{ marginTop: requiredFields.length > 0 ? 4 : 0 }}>
            {optionalFields.map((prop) => (
              <div key={prop.name}>{prop.content}</div>
            ))}
          </div>
        )}
      </div>
    </HideLabelContext.Provider>
  );
}
