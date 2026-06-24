import type * as Monaco from "monaco-editor";
import { publicAssetPath } from "./basePath";

let celConfigured = false;
let schemaPromise: Promise<CelSchemaIndex> | null = null;

export const celLanguage = "cel";

type CelCompletionEntry = {
  label: string;
  insertText?: string;
  detail: string;
  documentation?: string;
};

type JsonSchemaNode = {
  description?: string;
  type?: string | string[];
  properties?: Record<string, JsonSchemaNode>;
  additionalProperties?: JsonSchemaNode | boolean;
  items?: JsonSchemaNode;
  default?: unknown;
};

type CelSchemaIndex = {
  globals: CelCompletionEntry[];
  propertiesByPath: Record<string, CelCompletionEntry[]>;
};

const celMethods: readonly CelCompletionEntry[] = [
  { label: "size", insertText: "size()", detail: "CEL size" },
  { label: "contains", insertText: "contains($1)", detail: "CEL contains" },
  { label: "has", insertText: "has($1)", detail: "CEL presence test" },
  {
    label: "startsWith",
    insertText: "startsWith($1)",
    detail: "CEL string startsWith",
  },
  {
    label: "endsWith",
    insertText: "endsWith($1)",
    detail: "CEL string endsWith",
  },
  { label: "matches", insertText: "matches($1)", detail: "CEL regex match" },
  { label: "charAt", insertText: "charAt($1)", detail: "CEL string charAt" },
  { label: "indexOf", insertText: "indexOf($1)", detail: "CEL string indexOf" },
  {
    label: "lastIndexOf",
    insertText: "lastIndexOf($1)",
    detail: "CEL string lastIndexOf",
  },
  { label: "join", insertText: "join($1)", detail: "CEL string/header join" },
  {
    label: "lowerAscii",
    insertText: "lowerAscii()",
    detail: "CEL string lowerAscii",
  },
  {
    label: "upperAscii",
    insertText: "upperAscii()",
    detail: "CEL string upperAscii",
  },
  {
    label: "replace",
    insertText: "replace($1, $2)",
    detail: "CEL string replace",
  },
  {
    label: "regexReplace",
    insertText: "regexReplace($1, $2)",
    detail: "agentgateway regex replace",
  },
  {
    label: "stripPrefix",
    insertText: "stripPrefix($1)",
    detail: "CEL string stripPrefix",
  },
  {
    label: "stripSuffix",
    insertText: "stripSuffix($1)",
    detail: "CEL string stripSuffix",
  },
  { label: "trim", insertText: "trim()", detail: "CEL string trim" },
  { label: "split", insertText: "split($1)", detail: "CEL string split" },
  {
    label: "substring",
    insertText: "substring($1)",
    detail: "CEL string substring",
  },
  {
    label: "with",
    insertText: "with($1, $2)",
    detail: "agentgateway scoped binding helper",
  },
  { label: "filter", insertText: "filter($1, $2)", detail: "CEL list filter" },
  { label: "map", insertText: "map($1, $2)", detail: "CEL list map" },
  { label: "all", insertText: "all($1, $2)", detail: "CEL list all" },
  { label: "exists", insertText: "exists($1, $2)", detail: "CEL list exists" },
  {
    label: "exists_one",
    insertText: "exists_one($1, $2)",
    detail: "CEL list exists_one",
  },
  {
    label: "mapValues",
    insertText: "mapValues($1, $2)",
    detail: "agentgateway map value transform",
  },
  {
    label: "filterKeys",
    insertText: "filterKeys($1, $2)",
    detail: "agentgateway map key filter",
  },
  { label: "merge", insertText: "merge($1)", detail: "agentgateway map merge" },
  {
    label: "flatten",
    insertText: "flatten()",
    detail: "agentgateway flatten for logging/tracing",
  },
  {
    label: "flattenRecursive",
    insertText: "flattenRecursive()",
    detail: "agentgateway recursive flatten for logging/tracing",
  },
  {
    label: "redacted",
    insertText: "redacted()",
    detail: "agentgateway header view redaction",
  },
  {
    label: "raw",
    insertText: "raw()",
    detail: "agentgateway header raw values",
  },
  {
    label: "cookie",
    insertText: "cookie($1)",
    detail: "agentgateway request cookie lookup",
  },
  {
    label: "query",
    insertText: "query($1)",
    detail: "agentgateway query parameter lookup",
  },
  {
    label: "addQuery",
    insertText: "addQuery($1, $2)",
    detail: "agentgateway append query parameter",
  },
  {
    label: "setQuery",
    insertText: "setQuery($1, $2)",
    detail: "agentgateway set query parameter",
  },
  {
    label: "getFullYear",
    insertText: "getFullYear()",
    detail: "CEL timestamp getFullYear",
  },
  {
    label: "getMonth",
    insertText: "getMonth()",
    detail: "CEL timestamp getMonth",
  },
  {
    label: "getDayOfYear",
    insertText: "getDayOfYear()",
    detail: "CEL timestamp getDayOfYear",
  },
  {
    label: "getDayOfMonth",
    insertText: "getDayOfMonth()",
    detail: "CEL timestamp getDayOfMonth",
  },
  {
    label: "getDate",
    insertText: "getDate()",
    detail: "CEL timestamp getDate",
  },
  {
    label: "getDayOfWeek",
    insertText: "getDayOfWeek()",
    detail: "CEL timestamp getDayOfWeek",
  },
  {
    label: "getHours",
    insertText: "getHours()",
    detail: "CEL timestamp getHours",
  },
  {
    label: "getMinutes",
    insertText: "getMinutes()",
    detail: "CEL timestamp getMinutes",
  },
  {
    label: "getSeconds",
    insertText: "getSeconds()",
    detail: "CEL timestamp getSeconds",
  },
  {
    label: "getMilliseconds",
    insertText: "getMilliseconds()",
    detail: "CEL timestamp getMilliseconds",
  },
  { label: "family", insertText: "family()", detail: "Kubernetes IP family" },
  {
    label: "isUnspecified",
    insertText: "isUnspecified()",
    detail: "Kubernetes IP isUnspecified",
  },
  {
    label: "isLoopback",
    insertText: "isLoopback()",
    detail: "Kubernetes IP isLoopback",
  },
  {
    label: "isLinkLocalMulticast",
    insertText: "isLinkLocalMulticast()",
    detail: "Kubernetes IP isLinkLocalMulticast",
  },
  {
    label: "isLinkLocalUnicast",
    insertText: "isLinkLocalUnicast()",
    detail: "Kubernetes IP isLinkLocalUnicast",
  },
  {
    label: "isGlobalUnicast",
    insertText: "isGlobalUnicast()",
    detail: "Kubernetes IP isGlobalUnicast",
  },
  {
    label: "containsIP",
    insertText: "containsIP($1)",
    detail: "Kubernetes CIDR containsIP",
  },
  {
    label: "containsCIDR",
    insertText: "containsCIDR($1)",
    detail: "Kubernetes CIDR containsCIDR",
  },
  { label: "ip", insertText: "ip()", detail: "Kubernetes CIDR network IP" },
  { label: "masked", insertText: "masked()", detail: "Kubernetes CIDR masked" },
  {
    label: "prefixLength",
    insertText: "prefixLength()",
    detail: "Kubernetes CIDR prefixLength",
  },
] as const;

const celFunctions: readonly CelCompletionEntry[] = [
  {
    label: "json",
    insertText: "json($1)",
    detail: "Parse string or bytes as JSON",
  },
  {
    label: "toJson",
    insertText: "toJson($1)",
    detail: "Convert a CEL value to JSON",
  },
  {
    label: "unvalidatedJwtPayload",
    insertText: "unvalidatedJwtPayload($1)",
    detail: "Parse JWT payload without signature validation",
  },
  {
    label: "variables",
    insertText: "variables()",
    detail: "Expose all available variables as a value",
  },
  {
    label: "random",
    insertText: "random()",
    detail: "Generate a random float from 0.0 to 1.0",
  },
  {
    label: "default",
    insertText: "default($1, $2)",
    detail: "Use fallback when expression cannot resolve",
  },
  {
    label: "coalesce",
    insertText: "coalesce($1, $2)",
    detail: "First non-null successfully resolved expression",
  },
  {
    label: "fail",
    insertText: "fail($1)",
    detail: "Unconditionally fail an expression",
  },
  { label: "uuid", insertText: "uuid()", detail: "Generate a UUIDv4" },
  {
    label: "string",
    insertText: "string($1)",
    detail: "CEL string conversion",
  },
  { label: "bytes", insertText: "bytes($1)", detail: "CEL bytes conversion" },
  {
    label: "double",
    insertText: "double($1)",
    detail: "CEL double conversion",
  },
  { label: "int", insertText: "int($1)", detail: "CEL int conversion" },
  { label: "uint", insertText: "uint($1)", detail: "CEL uint conversion" },
  {
    label: "duration",
    insertText: "duration($1)",
    detail: "CEL duration conversion",
  },
  {
    label: "timestamp",
    insertText: "timestamp($1)",
    detail: "CEL timestamp conversion",
  },
  { label: "isIP", insertText: "isIP($1)", detail: "Kubernetes IP validation" },
  { label: "ip", insertText: "ip($1)", detail: "Kubernetes IP parser" },
  { label: "cidr", insertText: "cidr($1)", detail: "Kubernetes CIDR parser" },
  {
    label: "base64.encode",
    insertText: "base64.encode($1)",
    detail: "Base64 encode string or bytes",
  },
  {
    label: "base64.decode",
    insertText: "base64.decode($1)",
    detail: "Base64 decode to bytes",
  },
  {
    label: "url.encode",
    insertText: "url.encode($1)",
    detail: "Percent-encode URL component",
  },
  {
    label: "url.decode",
    insertText: "url.decode($1)",
    detail: "Percent-decode URL component",
  },
  {
    label: "sha1.encode",
    insertText: "sha1.encode($1)",
    detail: "SHA-1 hex digest",
  },
  {
    label: "sha256.encode",
    insertText: "sha256.encode($1)",
    detail: "SHA-256 hex digest",
  },
  {
    label: "md5.encode",
    insertText: "md5.encode($1)",
    detail: "MD5 hex digest",
  },
] as const;

const celNamespaceMethods: Record<string, readonly CelCompletionEntry[]> = {
  base64: [
    {
      label: "encode",
      insertText: "encode($1)",
      detail: "Base64 encode string or bytes",
    },
    {
      label: "decode",
      insertText: "decode($1)",
      detail: "Base64 decode to bytes",
    },
  ],
  url: [
    {
      label: "encode",
      insertText: "encode($1)",
      detail: "Percent-encode URL component",
    },
    {
      label: "decode",
      insertText: "decode($1)",
      detail: "Percent-decode URL component",
    },
  ],
  sha1: [
    { label: "encode", insertText: "encode($1)", detail: "SHA-1 hex digest" },
  ],
  sha256: [
    { label: "encode", insertText: "encode($1)", detail: "SHA-256 hex digest" },
  ],
  md5: [
    { label: "encode", insertText: "encode($1)", detail: "MD5 hex digest" },
  ],
};

export function configureCelMonaco(monaco: typeof Monaco) {
  if (celConfigured) return;
  celConfigured = true;

  monaco.languages.register({ id: celLanguage });
  monaco.languages.setLanguageConfiguration(celLanguage, {
    brackets: [
      ["(", ")"],
      ["[", "]"],
      ["{", "}"],
    ],
    autoClosingPairs: [
      { open: "(", close: ")" },
      { open: "[", close: "]" },
      { open: "{", close: "}" },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
    surroundingPairs: [
      { open: "(", close: ")" },
      { open: "[", close: "]" },
      { open: "{", close: "}" },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
  });
  monaco.languages.setMonarchTokensProvider(celLanguage, {
    keywords: ["true", "false", "null", "in"],
    operators: [
      "==",
      "!=",
      "<=",
      ">=",
      "<",
      ">",
      "&&",
      "||",
      "!",
      "+",
      "-",
      "*",
      "/",
      "%",
      "?",
      ":",
    ],
    tokenizer: {
      root: [
        [
          /[a-zA-Z_]\w*/,
          { cases: { "@keywords": "keyword", "@default": "identifier" } },
        ],
        [/\d+(\.\d+)?/, "number"],
        [/"([^"\\]|\\.)*$/, "string.invalid"],
        [/'([^'\\]|\\.)*$/, "string.invalid"],
        [/"/, "string", "@string_double"],
        [/'/, "string", "@string_single"],
        [/[{}()[\]]/, "@brackets"],
        [/[=!<>]=?|&&|\|\||[+\-*/%?:]/, "operator"],
        [/[;,.]/, "delimiter"],
      ],
      string_double: [
        [/[^\\"]+/, "string"],
        [/\\./, "string.escape"],
        [/"/, "string", "@pop"],
      ],
      string_single: [
        [/[^\\']+/, "string"],
        [/\\./, "string.escape"],
        [/'/, "string", "@pop"],
      ],
    },
  });

  monaco.languages.registerCompletionItemProvider(celLanguage, {
    triggerCharacters: ["."],
    async provideCompletionItems(model, position) {
      const schema = await loadCelSchemaIndex();
      const prefix = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: Math.max(1, position.column - 1),
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });
      const linePrefix = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });
      const isMemberCompletion = prefix === "." || /\.\w*$/.test(linePrefix);
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };
      const path = isMemberCompletion
        ? memberPathBeforeDot(linePrefix)
        : undefined;
      const namespace = path?.at(-1);
      const schemaEntries = path
        ? (schema.propertiesByPath[path.join(".")] ?? [])
        : [];
      const methodEntries =
        namespace && celNamespaceMethods[namespace]
          ? celNamespaceMethods[namespace]
          : isMemberCompletion
            ? celMethods
            : [];
      const entries = isMemberCompletion
        ? [...schemaEntries, ...methodEntries]
        : [...schema.globals, ...celFunctions];
      const seen = new Set<string>();
      const suggestions = entries
        .filter((entry) => {
          if (seen.has(entry.label)) return false;
          seen.add(entry.label);
          return true;
        })
        .map((entry) => ({
          label: entry.label,
          kind: isMemberCompletion
            ? monaco.languages.CompletionItemKind.Property
            : entry.insertText?.includes("(")
              ? monaco.languages.CompletionItemKind.Function
              : monaco.languages.CompletionItemKind.Variable,
          insertText: entry.insertText ?? entry.label,
          insertTextRules: entry.insertText?.includes("$")
            ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
            : undefined,
          detail: entry.detail,
          documentation: entry.documentation,
          range,
          sortText: `${entry.insertText?.includes("(") ? "1" : "0"}-${entry.label}`,
        }));
      return { suggestions };
    },
  });

  monaco.languages.registerHoverProvider(celLanguage, {
    async provideHover(model, position) {
      const schema = await loadCelSchemaIndex();
      const word = model.getWordAtPosition(position);
      if (!word) return null;
      const path = expressionPathAtPosition(model, position);
      const entries =
        path.length > 1
          ? (schema.propertiesByPath[path.slice(0, -1).join(".")] ?? [])
          : schema.globals;
      const entry =
        entries.find((candidate) => candidate.label === word.word) ??
        celFunctions.find((candidate) => candidate.label === word.word) ??
        celMethods.find((candidate) => candidate.label === word.word);
      if (!entry?.documentation && !entry?.detail) return null;
      return {
        range: new monaco.Range(
          position.lineNumber,
          word.startColumn,
          position.lineNumber,
          word.endColumn,
        ),
        contents: [
          { value: `\`${entry.label}\`` },
          { value: entry.documentation ?? entry.detail },
        ],
      };
    },
  });
}

export const celEditorOptions: Monaco.editor.IStandaloneEditorConstructionOptions =
  {
    acceptSuggestionOnCommitCharacter: false,
    copyWithSyntaxHighlighting: false,
    fontSize: 13,
    minimap: { enabled: false },
    quickSuggestions: { other: true, comments: false, strings: false },
    renderLineHighlight: "none",
    scrollBeyondLastLine: false,
    suggest: {
      showClasses: false,
      showColors: false,
      showConstructors: false,
      showDeprecated: false,
      showEnumMembers: true,
      showEnums: true,
      showEvents: false,
      showFiles: false,
      showFolders: false,
      showFunctions: true,
      showInterfaces: false,
      showIssues: false,
      showKeywords: false,
      showMethods: true,
      showModules: false,
      showOperators: false,
      showProperties: true,
      showReferences: false,
      showSnippets: false,
      showStructs: false,
      showTypeParameters: false,
      showUnits: false,
      showUsers: false,
      showValues: false,
      showVariables: true,
      showWords: false,
    },
    wordBasedSuggestions: "off",
  };

function memberPathBeforeDot(linePrefix: string) {
  const match = linePrefix.match(/([a-zA-Z_][\w]*(?:\.[a-zA-Z_][\w]*)*)\.\w*$/);
  return match?.[1]?.split(".");
}

function expressionPathAtPosition(
  model: Monaco.editor.ITextModel,
  position: Monaco.Position,
) {
  const linePrefix = model.getValueInRange({
    startLineNumber: position.lineNumber,
    startColumn: 1,
    endLineNumber: position.lineNumber,
    endColumn: position.column,
  });
  const match = linePrefix.match(/([a-zA-Z_][\w]*(?:\.[a-zA-Z_][\w]*)*)$/);
  return match?.[1]?.split(".") ?? [];
}

function loadCelSchemaIndex() {
  schemaPromise ??= fetch(publicAssetPath("cel-schema.json"))
    .then((response) =>
      response.ok
        ? (response.json() as Promise<JsonSchemaNode>)
        : { properties: {} },
    )
    .then(buildSchemaIndex)
    .catch(() => ({ globals: [], propertiesByPath: {} }));
  return schemaPromise;
}

function buildSchemaIndex(schema: JsonSchemaNode): CelSchemaIndex {
  const propertiesByPath: Record<string, CelCompletionEntry[]> = {};
  const globals = Object.entries(schema.properties ?? {}).map(([name, node]) =>
    schemaEntry(name, node),
  );

  const visit = (path: string, node: JsonSchemaNode) => {
    const properties = node.properties ?? {};
    propertiesByPath[path] = Object.entries(properties).map(([name, child]) =>
      schemaEntry(name, child),
    );
    for (const [name, child] of Object.entries(properties)) {
      if (child.properties) visit(`${path}.${name}`, child);
    }
  };

  for (const [name, node] of Object.entries(schema.properties ?? {})) {
    visit(name, node);
  }

  return { globals, propertiesByPath };
}

function schemaEntry(label: string, node: JsonSchemaNode): CelCompletionEntry {
  const type = Array.isArray(node.type)
    ? node.type.filter((entry) => entry !== "null").join(" | ")
    : node.type;
  const detail = type ? `CEL ${type}` : "CEL value";
  return {
    label,
    detail,
    documentation: node.description,
  };
}
