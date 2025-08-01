import { useEffect, useState } from "react";
import { isXdsMode, subscribeXdsMode } from "@/lib/api";

/**
 * React hook that returns whether AgentGateway is running in XDS-managed mode.
 * It subscribes to updates so that components re-render automatically if the
 * mode changes during the session.
 */
export function useXdsMode(): boolean {
  const [xds, setXds] = useState<boolean>(isXdsMode());

  useEffect(() => {
    return subscribeXdsMode(setXds);
  }, []);

  return xds;
}
