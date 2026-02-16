
import { useState, useEffect } from "react";
import { AppSidebar } from "@/components/app-sidebar";
import { useServer } from "@/lib/server-context";
import { useLoading } from "@/lib/loading-context";
import { useWizard } from "@/lib/wizard-context";

export function SidebarWrapper() {
  const { isConnected } = useServer();
  const { setIsLoading } = useLoading();
  const [activeView, setActiveView] = useState("home");
  const { isWizardVisible } = useWizard();

  // Update loading state based on connection status
  useEffect(() => {
    if (isConnected) {
      setIsLoading(false);
    }
  }, [isConnected, setIsLoading]);

  // Don't render the sidebar if the wizard is visible
  if (isWizardVisible) {
    return null;
  }

  return <AppSidebar activeView={activeView} setActiveView={setActiveView} />;
}
