import type { ButtonProps } from "antd";
import { Button, Tooltip } from "antd";
import { useXdsMode } from "../api/hooks";

export function XdsAwareButton(props: ButtonProps) { 
    const { xdsMode } = useXdsMode();

    return (
        <Tooltip title={xdsMode ? "Configuration is managed by xDS" : ""}>
            <Button 
                {...props} 
                disabled={props.disabled || xdsMode}
            />
        </Tooltip>
    );
}