import { Drawer, Typography } from "antd";
import type { TopLevelEditTarget } from "./TopLevelEditForm";
import { TopLevelEditForm } from "./TopLevelEditForm";

const { Text } = Typography;

interface TopLevelDrawerProps {
  target: TopLevelEditTarget | null;
  onClose: () => void;
  onSaved: () => void;
}

const LABELS: Record<string, string> = {
  llm: "LLM Configuration",
  mcp: "MCP Configuration",
  frontendPolicies: "Frontend Policies",
  backend: "Backend",
  policy: "Policy",
};

export function TopLevelDrawer({
  target,
  onClose,
  onSaved,
}: TopLevelDrawerProps) {
  const label = target ? LABELS[target.type] || target.type : "";
  const isEditing = target?.initialData !== undefined;
  const title = target ? `${isEditing ? "Edit" : "New"} ${label}` : "";

  return (
    <Drawer
      title={<Text strong>{title}</Text>}
      open={!!target}
      onClose={onClose}
      width="min(92vw, 1040px)"
      destroyOnClose
    >
      {target && (
        <TopLevelEditForm
          key={`${target.type}-${target.initialData ? 'edit' : 'new'}`}
          target={target}
          onSaved={() => {
            onSaved();
            onClose();
          }}
          onCancel={onClose}
        />
      )}
    </Drawer>
  );
}
