import { useState } from "react";

interface UseConfirmModalOptions {
  title: string;
  content: string;
  onConfirm: () => void | Promise<void>;
  confirmText?: string;
  danger?: boolean;
}

interface UseConfirmModalReturn {
  modalProps: {
    title: string;
    content: string;
    open: boolean;
    onConfirm: () => void | Promise<void>;
    onCancel: () => void;
    confirmText?: string;
    danger?: boolean;
    confirmLoading: boolean;
  };
  confirm: (options: UseConfirmModalOptions) => void;
  close: () => void;
}

export const useConfirmModal = (): UseConfirmModalReturn => {
  const [isOpen, setIsOpen] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [currentOptions, setCurrentOptions] =
    useState<UseConfirmModalOptions | null>(null);

  const confirm = (options: UseConfirmModalOptions) => {
    setCurrentOptions(options);
    setIsOpen(true);
  };

  const close = () => {
    setIsOpen(false);
    setCurrentOptions(null);
    setIsLoading(false);
  };

  const handleConfirm = async () => {
    if (!currentOptions) return;

    setIsLoading(true);
    try {
      await currentOptions.onConfirm();
      close();
    } catch (error) {
      // Error handling is done in the onConfirm function
      setIsLoading(false);
    }
  };

  const handleCancel = () => {
    close();
  };

  return {
    modalProps: {
      title: currentOptions?.title || "",
      content: currentOptions?.content || "",
      open: isOpen,
      onConfirm: handleConfirm,
      onCancel: handleCancel,
      confirmText: currentOptions?.confirmText,
      danger: currentOptions?.danger,
      confirmLoading: isLoading,
    },
    confirm,
    close,
  };
};
