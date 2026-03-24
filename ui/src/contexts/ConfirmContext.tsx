import type { ReactNode } from "react";
import { createContext, useContext, useState } from "react";
import { ConfirmModal } from "../components/ConfirmModal";

interface ConfirmOptions {
  title: string;
  content: string;
  onConfirm: () => void | Promise<void>;
  confirmText?: string;
  danger?: boolean;
}

interface ConfirmContextType {
  confirm: (options: ConfirmOptions) => void;
}

const ConfirmContext = createContext<ConfirmContextType | null>(null);

interface ConfirmProviderProps {
  children: ReactNode;
}

export const ConfirmProvider = ({ children }: ConfirmProviderProps) => {
  const [isOpen, setIsOpen] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [currentOptions, setCurrentOptions] = useState<ConfirmOptions | null>(
    null,
  );

  const confirm = (options: ConfirmOptions) => {
    setCurrentOptions(options);
    setIsOpen(true);
  };

  const handleConfirm = async () => {
    if (!currentOptions) return;

    setIsLoading(true);
    try {
      await currentOptions.onConfirm();
      setIsOpen(false);
      setCurrentOptions(null);
    } catch (error) {
      // Error handling is done in the onConfirm function
      setIsLoading(false);
    }
  };

  const handleCancel = () => {
    setIsOpen(false);
    setCurrentOptions(null);
    setIsLoading(false);
  };

  const modalProps = {
    title: currentOptions?.title || "",
    content: currentOptions?.content || "",
    open: isOpen,
    onConfirm: handleConfirm,
    onCancel: handleCancel,
    confirmText: currentOptions?.confirmText,
    danger: currentOptions?.danger,
    confirmLoading: isLoading,
  };

  return (
    <ConfirmContext.Provider value={{ confirm }}>
      {children}
      <ConfirmModal {...modalProps} />
    </ConfirmContext.Provider>
  );
};

export const useConfirm = () => {
  const context = useContext(ConfirmContext);
  if (!context) {
    throw new Error("useConfirm must be used within a ConfirmProvider");
  }
  return context.confirm;
};
