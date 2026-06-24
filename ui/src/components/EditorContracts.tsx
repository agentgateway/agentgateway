import type { ReactNode } from "react";
import type { SchemaHelp } from "../schemaHelp";
import { StatusBanner, YamlBlock } from "./Primitives";
import { SchemaHelpPanel } from "./SchemaHelpPanel";

export function UnsupportedYamlFallback(props: {
  title: string;
  children: ReactNode;
  value: unknown;
  schema?: unknown;
  help?: SchemaHelp;
}) {
  return (
    <div className="policy-editor-stack">
      <StatusBanner state="warn" title={props.title}>
        {props.children}
      </StatusBanner>
      {props.schema && props.help ? (
        <SchemaHelpPanel schema={props.schema} help={props.help} />
      ) : null}
      <YamlBlock value={props.value} />
    </div>
  );
}
