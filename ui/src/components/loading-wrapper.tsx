import { ReactNode } from "react";
import { LoadingProvider } from "@/lib/loading-context";

export function LoadingWrapper({ children }: { children: ReactNode }) {
  return (
    <LoadingProvider>
      <main className="flex-1 overflow-auto">{children}</main>
    </LoadingProvider>
  );
}
