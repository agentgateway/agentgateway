export type TemplateKey = "empty" | "http";

export interface MonacoEditorProps {
  value: string;
  onChange: (value: string | undefined) => void;
  language: string;
  height: string;
  theme: string;
  options?: Record<string, unknown>;
  onEvaluate?: () => void;
}

export interface Example {
  name: string;
  expr: string;
}

export interface ExpressionTemplate {
  name: string;
  description: string;
  expression: string;
  context: unknown;
}
