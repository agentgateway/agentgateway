import { Modal } from "antd";
import type { ReactNode } from "react";

interface ConfirmModalProps {
  title: string;
  content: ReactNode;
  open: boolean;
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
  confirmText?: string;
  cancelText?: string;
  confirmLoading?: boolean;
  danger?: boolean;
}

export const ConfirmModal = ({
  title,
  content,
  open,
  onConfirm,
  onCancel,
  confirmText = "Confirm",
  cancelText = "Cancel",
  confirmLoading = false,
  danger = false,
}: ConfirmModalProps) => {
  const footer = (
    <div style={{ display: "flex", justifyContent: "flex-end", gap: "8px" }}>
      <span
        onClick={onCancel}
        style={{
          padding: "6px 16px",
          cursor: "pointer",
          color: "#000000d9",
          backgroundColor: "#fff",
          border: "1px solid #d9d9d9",
          borderRadius: "6px",
        }}
      >
        {cancelText}
      </span>
      <span
        onClick={onConfirm}
        style={{
          padding: "6px 16px",
          cursor: confirmLoading ? "not-allowed" : "pointer",
          color: danger ? "#fff" : "#fff",
          backgroundColor: danger ? "#ff4d4f" : "#1890ff",
          border: "none",
          borderRadius: "6px",
          opacity: confirmLoading ? 0.6 : 1,
        }}
      >
        {confirmLoading ? "Loading..." : confirmText}
      </span>
    </div>
  );

  return (
    <Modal
      title={title}
      open={open}
      onCancel={onCancel}
      footer={footer}
      confirmLoading={confirmLoading}
    >
      {content}
    </Modal>
  );
};
