import { YamlBlock } from "../components/Primitives";

export function ResultingYaml(props: { value: unknown; label?: string }) {
  return (
    <details className="schema-details">
      <summary>{props.label ?? "Resulting YAML"}</summary>
      <YamlBlock value={props.value} />
    </details>
  );
}
