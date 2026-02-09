"use client";

import { useState } from "react";
import {
  Sidebar,
  SidebarContent,
  SidebarHeader,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarMenuBadge,
  SidebarSeparator,
} from "@/components/ui/sidebar";
import { Button } from "@/components/ui/button";
import { AgentgatewayLogo } from "@/components/agentgateway-logo";
import { ThemeToggle } from "@/components/theme-toggle";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Loader2,
  Home,
  Shield,
  Headphones,
  Server,
  Code,
  Settings,
  Route,
  Calculator,
} from "lucide-react";
import { useRouter, usePathname } from "next/navigation";
import { useWizard } from "@/lib/wizard-context";
import { toast } from "sonner";
import { useServer } from "@/lib/server-context";

interface AppSidebarProps {
  activeView: string;
  setActiveView: (view: string) => void;
}

export function AppSidebar({ setActiveView }: AppSidebarProps) {
  const [showRestartDialog, setShowRestartDialog] = useState(false);
  const router = useRouter();
  const pathname = usePathname();
  const { restartWizard, isRestartingWizard } = useWizard();
  const { listeners } = useServer();

  const routeCount = listeners.reduce((count, listener) => {
    const httpRoutes = listener.routes?.length || 0;
    const tcpRoutes = listener.tcpRoutes?.length || 0;
    return count + httpRoutes + tcpRoutes;
  }, 0);

  const backendCount = listeners.reduce((count, listener) => {
    let backendSum = 0;

    listener.routes?.forEach((route) => {
      backendSum += route.backends?.length || 0;
    });

    listener.tcpRoutes?.forEach((tcpRoute) => {
      backendSum += tcpRoute.backends?.length || 0;
    });
    return count + backendSum;
  }, 0);

  const handleRestartWizard = () => {
    setShowRestartDialog(true);
  };

  const confirmRestartWizard = async () => {
    try {
      await restartWizard();
      navigateTo("/");
    } catch (error) {
      console.error("Error restarting wizard:", error);
      toast.error(error instanceof Error ? error.message : "Failed to restart wizard");
    } finally {
      setShowRestartDialog(false);
    }
  };

  const navigateTo = (path: string) => {
    router.push(path);
    setActiveView(path.split("/").pop() || "home");
  };

  return (
    <Sidebar>
      <SidebarHeader className="border-b">
        <div className="p-2 flex items-center justify-center mb-2">
          <AgentgatewayLogo className="h-10 w-auto" />
          <span className="text-2xl ml-2 font-bold">agentgateway</span>
        </div>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Navigation</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton
                  tooltip="Home"
                  isActive={pathname === "/"}
                  onClick={() => navigateTo("/")}
                  aria-label="Home"
                >
                  <Home className="h-4 w-4" />
                  <span>Home</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  tooltip="Port Binds & Listeners"
                  isActive={pathname === "/listeners"}
                  onClick={() => navigateTo("/listeners")}
                  aria-label="Port Binds & Listeners"
                >
                  <Headphones className="h-4 w-4" />
                  <span>Listeners</span>
                  <SidebarMenuBadge>{listeners.length}</SidebarMenuBadge>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  tooltip="Routes"
                  isActive={pathname === "/routes"}
                  onClick={() => navigateTo("/routes")}
                  aria-label="Routes"
                >
                  <Route className="h-4 w-4" />
                  <span>Routes</span>
                  <SidebarMenuBadge>{routeCount}</SidebarMenuBadge>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  tooltip="Backends"
                  isActive={pathname === "/backends"}
                  onClick={() => navigateTo("/backends")}
                  aria-label="Backends"
                >
                  <Server className="h-4 w-4" />
                  <span>Backends</span>
                  <SidebarMenuBadge>{backendCount}</SidebarMenuBadge>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  tooltip="Policies"
                  isActive={pathname === "/policies"}
                  onClick={() => navigateTo("/policies")}
                  aria-label="Policies"
                >
                  <Shield className="h-4 w-4" />
                  <span>Policies</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  tooltip="Playground"
                  isActive={pathname === "/playground"}
                  onClick={() => navigateTo("/playground")}
                  aria-label="Playground"
                >
                  <Code className="h-4 w-4" />
                  <span>Playground</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton
                  tooltip="CEL Playground"
                  isActive={pathname === "/cel"}
                  onClick={() => navigateTo("/cel")}
                  aria-label="CEL Playground"
                >
                  <Calculator className="h-4 w-4" />
                  <span>CEL Playground</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
        <SidebarSeparator />
      </SidebarContent>

      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              tooltip="Restart Setup Wizard"
              onClick={handleRestartWizard}
              aria-label="Restart Setup Wizard"
            >
              <Settings className="h-4 w-4" />
              <span>Restart Setup</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton tooltip="Toggle Theme" aria-label="Toggle Theme">
              <ThemeToggle asChild />
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>

      <Dialog open={showRestartDialog} onOpenChange={setShowRestartDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Restart Setup Wizard</DialogTitle>
            <DialogDescription>
              Are you sure you want to restart the setup wizard? This will reset all your current
              configuration settings.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowRestartDialog(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={confirmRestartWizard}
              disabled={isRestartingWizard}
            >
              {isRestartingWizard ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Restarting...
                </>
              ) : (
                "Restart"
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Sidebar>
  );
}
