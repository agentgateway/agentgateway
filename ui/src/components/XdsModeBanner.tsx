import { Alert } from "antd";
import { useXdsMode } from "../api/hooks";

export function XdsModeBanner() { 
    const { xdsMode, xdsAddress} = useXdsMode();

    if (!xdsMode) return null;
    
    return (
        <Alert 
            type="info"
            banner
            showIcon
            message="Configuration is managed by xDS"
            description={
                xdsAddress 
                    ? `This agentgateway is receiving its configuration from ${xdsAddress}.  Edits are disabled.` 
                    : "This agentgateway is receiving its configuration from a remote control plane.  Edits are disabled."
            }
        />
    );
}