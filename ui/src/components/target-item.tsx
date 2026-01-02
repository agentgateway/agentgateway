"use client";

import { TargetType, TargetWithType } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Edit2, Trash2 } from "lucide-react";
import { TooltipProvider, Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useXdsMode } from "@/hooks/use-xds-mode";

interface TargetItemProps {
  target: TargetWithType;
  index: number;
  onDelete: (index: number) => void;
  onEdit: (target: TargetWithType) => void;
  isUpdating: boolean;
}

export const getTargetType = (target: TargetWithType): TargetType => {
  switch (target.type) {
    case "a2a":
      return "a2a";
    case "mcp":
      return "mcp";
    case "openapi":
      return "openapi";
    case "stdio":
      return "stdio";
    case "sse":
      return "sse";
    default:
      return "unknown";
  }
};

export default function TargetItem({
  target,
  index,
  onDelete,
  onEdit,
  isUpdating,
}: TargetItemProps) {
  const targetType = getTargetType(target as TargetWithType);
  const xds = useXdsMode();

  return (
    <div className="flex items-center justify-between w-full">
      <div className="flex items-center space-x-2">
        <Badge variant="outline">{targetType}</Badge>
        <div>
          <div className="font-medium">{target.name}</div>
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="text-xs text-muted-foreground truncate max-w-[400px]">
                  {target.mcp && `${target.mcp.host}`}
                  {target.sse && `${target.sse.host}`}
                  {target.stdio && `${target.stdio.cmd} ${target.stdio.args?.join(" ")}`}
                  {target.openapi &&
                    `${target.openapi.host}${
                      typeof target.openapi.port === "number" ? `:${target.openapi.port}` : ""
                    }`}
                  {target.a2a && `${target.a2a.host}:${target.a2a.port}${target.a2a.path}`}
                </div>
              </TooltipTrigger>
              <TooltipContent>
                {target.mcp && `${target.mcp.host}`}
                {target.sse && `${target.sse.host}`}
                {target.stdio && `${target.stdio.cmd} ${target.stdio.args?.join(" ")}`}
                {target.openapi &&
                  `${target.openapi.host}${
                    typeof target.openapi.port === "number" ? `:${target.openapi.port}` : ""
                  }`}
                {target.a2a && `${target.a2a.host}:${target.a2a.port}${target.a2a.path}`}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </div>
      </div>
      <div className="flex items-center gap-2">
        <div className="flex flex-wrap items-center gap-1">
          {target.listeners === undefined ? (
            <Badge className="text-xs">All listeners</Badge>
          ) : target.listeners.length === 0 ? (
            <Badge className="text-xs bg-muted">No listeners</Badge>
          ) : (
            target.listeners.map((listener) => (
              <Badge key={listener} className="text-xs">
                {listener}
              </Badge>
            ))
          )}
        </div>
        <Button
          variant="ghost"
          size="icon"
          onClick={() => onDelete(index)}
          className={`h-8 w-8 ml-2 text-muted-foreground hover:bg-primary/20 flex-shrink-0 ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
          disabled={isUpdating || xds}
        >
          <Trash2 className="h-4 w-4" />
        </Button>

        <Button
          variant="ghost"
          size="icon"
          onClick={() => onEdit(target)}
          className={`h-8 w-8 ml-2 text-muted-foreground hover:bg-primary/20 flex-shrink-0 ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
          disabled={isUpdating || xds}
        >
          <Edit2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
