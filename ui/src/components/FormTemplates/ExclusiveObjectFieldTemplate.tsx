/**
 * ExclusiveObjectFieldTemplate
 *
 * Extends CollapsibleObjectFieldTemplate with support for mutually-exclusive
 * field groups (oneOf patterns not encoded in JSON Schema).
 *
 * Consumers pass an `exclusiveFormContext` in the RJSF <Form formContext={…}>
 * prop.  At the top-level object (nestingLevel 0) this template renders a
 * labeled Select dropdown for each group in place of a plain property row, and
 * hides the inactive group fields so the user only sees the chosen one.
 *
 * At any deeper nesting level the template falls through to
 * CollapsibleObjectFieldTemplate unchanged.
 */
import type { ObjectFieldTemplateProps } from "@rjsf/utils";
import { Select, Typography } from "antd";
import { useMemo } from "react";
import { CollapsibleObjectFieldTemplate } from "./CollapsibleObjectFieldTemplate";

// ---------------------------------------------------------------------------
// Shared context type (must match what NodeEditDrawer passes as formContext)
// ---------------------------------------------------------------------------

export interface ExclusiveOption {
  fieldKey: string;
  label: string;
}

export interface ExclusiveGroup {
  groupLabel: string;
  options: ExclusiveOption[];
  defaultKey: string;
}

export interface ExclusiveFormContext {
  exclusiveGroups?: ExclusiveGroup[];
  activeGroupKeys?: Record<string, string>;
  onGroupKeyChange?: (group: ExclusiveGroup, newKey: string) => void;
}

// ---------------------------------------------------------------------------
// Template
// ---------------------------------------------------------------------------

const { Text } = Typography;

export function ExclusiveObjectFieldTemplate(props: ObjectFieldTemplateProps) {
  const { properties, uiSchema, formContext } = props;

  const nestingLevel = (uiSchema?.["ui:nestingLevel"] as number) || 0;
  const ctx = formContext as ExclusiveFormContext | undefined;
  const groups = useMemo(() => ctx?.exclusiveGroups ?? [], [ctx]);
  const activeKeys = useMemo(() => ctx?.activeGroupKeys ?? {}, [ctx]);

  // Set of inactive field names — computed before any conditional returns so
  // hooks are always called in the same order.
  const inactiveFields = useMemo(() => {
    const s = new Set<string>();
    for (const group of groups) {
      const activeKey = activeKeys[group.groupLabel] ?? group.defaultKey;
      for (const opt of group.options) {
        if (opt.fieldKey !== activeKey) s.add(opt.fieldKey);
      }
    }
    return s;
  }, [groups, activeKeys]);

  // Map from first-option fieldKey → group (anchor where selector is injected)
  const anchorGroupMap = useMemo(() => {
    const m = new Map<string, ExclusiveGroup>();
    for (const group of groups) {
      m.set(group.options[0].fieldKey, group);
    }
    return m;
  }, [groups]);

  // At deeper nesting or when there are no groups, delegate unchanged.
  if (nestingLevel > 0 || !groups.length) {
    return <CollapsibleObjectFieldTemplate {...props} />;
  }

  // Build synthetic properties:
  //  • inactive group fields are removed
  //  • the anchor field of each group has the Select injected above its content
  const syntheticProperties = properties
    .filter((p) => !inactiveFields.has(p.name))
    .map((p) => {
      const anchorGroup = anchorGroupMap.get(p.name);
      if (!anchorGroup) return p;

      const activeKey =
        activeKeys[anchorGroup.groupLabel] ?? anchorGroup.defaultKey;

      return {
        ...p,
        content: (
          <div key={p.name}>
            <div style={{ marginBottom: 8 }}>
              <Text
                style={{
                  display: "block",
                  marginBottom: 4,
                  fontSize: 13,
                  fontWeight: 500,
                }}
              >
                {anchorGroup.groupLabel}
              </Text>
              <Select
                value={activeKey}
                onChange={(val) => ctx?.onGroupKeyChange?.(anchorGroup, val)}
                style={{ width: "100%" }}
                options={anchorGroup.options.map((opt) => ({
                  value: opt.fieldKey,
                  label: opt.label,
                }))}
              />
            </div>
            {p.content}
          </div>
        ),
      };
    });

  return (
    <CollapsibleObjectFieldTemplate
      {...props}
      properties={syntheticProperties}
    />
  );
}
