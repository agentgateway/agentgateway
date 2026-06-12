import { DeleteOutlined, PlusOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import type { FieldProps } from "@rjsf/utils";
import { Button, Input, Typography } from "antd";
import { useCallback, useEffect, useRef, useState } from "react";

const { Text } = Typography;

const Container = styled.div`
  margin-bottom: var(--spacing-md);
`;

const Row = styled.div`
  display: flex;
  gap: var(--spacing-sm);
  align-items: flex-start;
  margin-bottom: var(--spacing-sm);
`;

const KeyInput = styled(Input)`
  flex: 0 0 200px;
  min-width: 120px;
`;

const ValueInput = styled(Input)`
  flex: 1;
  min-width: 0;
`;

const Header = styled.div`
  display: flex;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-xs);
  padding-left: 2px;
`;

const HeaderLabel = styled(Text)`
  font-size: var(--font-size-sm);
  font-weight: var(--font-weight-medium);
  color: var(--color-text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.03em;
`;

interface Entry {
  id: number;
  key: string;
  value: string;
}

/**
 * Custom RJSF field for editing Record<string, string> maps.
 *
 * Use in uiSchema: `"ui:field": "keyValueMap"`
 *
 * Renders a clean key-value pair editor with labeled columns,
 * add/remove buttons, and contextual placeholders.
 */
export function KeyValueMapField(props: FieldProps) {
  const { formData, onChange, disabled, readonly, uiSchema, schema } = props;

  const isDisabled = disabled || readonly;
  const shouldHideLabel = uiSchema?.["ui:label"] === false;
  const keyPlaceholder =
    (uiSchema?.["ui:keyPlaceholder"] as string) ?? "key";
  const valuePlaceholder =
    (uiSchema?.["ui:valuePlaceholder"] as string) ?? "value";

  const nextId = useRef(0);

  // Internal state: array of { id, key, value } with stable IDs
  const [entries, setEntries] = useState<Entry[]>(() =>
    Object.entries(formData ?? {}).map(([k, v]) => ({
      id: nextId.current++,
      key: k,
      value: String(v ?? ""),
    })),
  );

  // Sync from external formData changes (e.g., form reset)
  const lastEmitted = useRef<Record<string, string> | undefined>(undefined);
  useEffect(() => {
    const incoming = formData ?? {};
    // Skip if this is the same object we emitted
    if (incoming === lastEmitted.current) return;
    // Check if the data actually differs from our entries
    const currentKeys = entries.map((e) => e.key);
    const incomingEntries = Object.entries(incoming);
    if (
      incomingEntries.length === entries.length &&
      incomingEntries.every(
        ([k, v], i) => currentKeys[i] === k && entries[i].value === String(v ?? ""),
      )
    ) {
      return;
    }
    setEntries(
      incomingEntries.map(([k, v]) => ({
        id: nextId.current++,
        key: k,
        value: String(v ?? ""),
      })),
    );
  }, [formData]); // eslint-disable-line react-hooks/exhaustive-deps

  const emitChange = useCallback(
    (updated: Entry[]) => {
      const obj: Record<string, string> = {};
      for (const e of updated) {
        obj[e.key] = e.value;
      }
      lastEmitted.current = obj;
      onChange(Object.keys(obj).length > 0 ? obj : undefined);
    },
    [onChange],
  );

  const updateKey = useCallback(
    (id: number, newKey: string) => {
      setEntries((prev) => {
        const updated = prev.map((e) =>
          e.id === id ? { ...e, key: newKey } : e,
        );
        emitChange(updated);
        return updated;
      });
    },
    [emitChange],
  );

  const updateValue = useCallback(
    (id: number, newValue: string) => {
      setEntries((prev) => {
        const updated = prev.map((e) =>
          e.id === id ? { ...e, value: newValue } : e,
        );
        emitChange(updated);
        return updated;
      });
    },
    [emitChange],
  );

  const addEntry = useCallback(() => {
    setEntries((prev) => {
      const updated = [...prev, { id: nextId.current++, key: "", value: "" }];
      emitChange(updated);
      return updated;
    });
  }, [emitChange]);

  const removeEntry = useCallback(
    (id: number) => {
      setEntries((prev) => {
        const updated = prev.filter((e) => e.id !== id);
        emitChange(updated);
        return updated;
      });
    },
    [emitChange],
  );

  return (
    <Container>
      {schema.title && !shouldHideLabel && (
        <Text strong style={{ display: "block", marginBottom: 8 }}>
          {schema.title}
        </Text>
      )}
      {schema.description && (
        <Typography.Paragraph
          type="secondary"
          style={{ marginBottom: 8, fontSize: 13 }}
        >
          {schema.description}
        </Typography.Paragraph>
      )}

      {entries.length > 0 && (
        <Header>
          <HeaderLabel style={{ flex: "0 0 200px" }}>Key</HeaderLabel>
          <HeaderLabel style={{ flex: 1 }}>Value</HeaderLabel>
          <div style={{ width: 32 }} />
        </Header>
      )}

      {entries.map((entry) => (
        <Row key={entry.id}>
          <KeyInput
            placeholder={keyPlaceholder}
            value={entry.key}
            onChange={(e) => updateKey(entry.id, e.target.value)}
            disabled={isDisabled}
          />
          <ValueInput
            placeholder={valuePlaceholder}
            value={entry.value}
            onChange={(e) => updateValue(entry.id, e.target.value)}
            disabled={isDisabled}
          />
          <Button
            type="text"
            danger
            icon={<DeleteOutlined />}
            onClick={() => removeEntry(entry.id)}
            disabled={isDisabled}
          />
        </Row>
      ))}

      <Button
        type="dashed"
        icon={<PlusOutlined />}
        onClick={addEntry}
        disabled={isDisabled}
        style={{ width: "100%" }}
      >
        Add Entry
      </Button>
    </Container>
  );
}
