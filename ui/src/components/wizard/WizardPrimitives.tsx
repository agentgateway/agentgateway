import styled from "@emotion/styled";
import { Form, Input } from "antd";
import { Check, Copy } from "lucide-react";
import { useCallback, useState } from "react";
import { Tooltip } from "antd";
import { Card } from "antd";

// ── Layout primitives ─────────────────────────────────────────────────────────

export const StepTitle = styled.h2`
  font-size: 20px;
  font-weight: 600;
  color: var(--color-text-base);
  margin: 0 0 var(--spacing-sm) 0;
`;

export const StepDescription = styled.p`
  color: var(--color-text-secondary);
  font-size: 14px;
  margin: 0 0 var(--spacing-xl) 0;
`;

export const Actions = styled.div`
  display: flex;
  justify-content: space-between;
  margin-top: var(--spacing-xl);
`;

// ── Numbered step list ────────────────────────────────────────────────────────

export const StepList = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
  margin: 0 0 var(--spacing-xl) 0;
`;

export const StepRow = styled.div`
  display: flex;
  align-items: flex-start;
  gap: var(--spacing-md);
  background: var(--color-bg-elevated, #fafafa);
  border: 1px solid var(--color-border);
  border-left: 3px solid var(--ant-color-primary, #6941c6);
  border-radius: var(--border-radius-md);
  padding: var(--spacing-md) var(--spacing-lg);
`;

export const StepNumber = styled.div`
  flex-shrink: 0;
  width: 24px;
  height: 24px;
  border-radius: 50%;
  background: var(--ant-color-primary, #6941c6);
  color: #fff;
  font-size: 12px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
`;

export const StepContent = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
  font-size: 14px;
  color: var(--color-text-base);
  line-height: 1.6;
`;

// ── Terminal / copy-command ───────────────────────────────────────────────────

export const TerminalBlock = styled.code`
  display: inline-block;
  background: #8b8b8b;
  color: #ffffff;
  border-radius: var(--border-radius-sm);
  border: none;
  padding: 4px 10px;
  font-family: monospace;
  font-size: 13px;
`;

export const CommandWrapper = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  margin-top: 2px;
`;

export const CopyButton = styled.button`
  flex-shrink: 0;
  background: none;
  border: none;
  cursor: pointer;
  color: #888;
  padding: 2px 4px;
  border-radius: var(--border-radius-sm);
  display: flex;
  align-items: center;

  &:hover {
    color: #d4d4d4;
    background: rgba(0, 0, 0, 0.06);
  }
`;

export function CopyableCommand({ children }: { children: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(children);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [children]);

  return (
    <CommandWrapper>
      <TerminalBlock>{children}</TerminalBlock>
      <Tooltip title={copied ? "Copied!" : "Copy"}>
        <CopyButton onClick={handleCopy}>
          {copied ? <Check size={14} color="#52c41a" /> : <Copy size={14} />}
        </CopyButton>
      </Tooltip>
    </CommandWrapper>
  );
}

// ── Form field layout ─────────────────────────────────────────────────────────

export const FieldFormItem = styled(Form.Item)`
  .ant-form-item-label > label {
    align-items: baseline;
  }
`;

export const FieldFormTitle = styled.div`
  font-weight: 600;
`;

export const FieldFormDescription = styled.div`
  color: var(--color-text-tertiary);
  font-size: 12px;
  font-style: italic;
  margin: 0 0 var(--spacing-xs) 0;
`;

export const StyledInput = styled(Input)`
  width: 100%;
  border: 1px solid #d9d9d9 !important;
`;

// ── Option card grid (used in select steps) ───────────────────────────────────

export const CardGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: var(--spacing-lg);
`;

export const OptionCard = styled(Card)<{ $selected: boolean }>`
  cursor: pointer;
  box-shadow: ${({ $selected }) =>
    $selected ? "0 0 0 2px var(--color-primary)" : "none"};
  background: ${({ $selected }) =>
    $selected ? "var(--color-primary-bg)" : "inherit"};
  transition: box-shadow 0.2s, background 0.2s;

  &:hover {
    box-shadow: ${({ $selected }) =>
      $selected
        ? "0 0 0 2px var(--color-primary)"
        : "0 0 0 2px var(--color-primary-hover)"};
  }
`;

export const CardLabel = styled.div<{ $selected: boolean }>`
  font-weight: 600;
  font-size: 15px;
  color: ${({ $selected }) =>
    $selected ? "var(--color-primary)" : "var(--color-text-base)"};
  margin-bottom: var(--spacing-xs);
`;

export const CardSubtext = styled.div`
  font-size: 13px;
  color: var(--color-text-secondary);
`;
