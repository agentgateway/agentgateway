import Editor from "@monaco-editor/react";
import { Link } from "@tanstack/react-router";
import { Braces, Pencil, Plus, Save, Trash2 } from "lucide-react";
import { useState } from "react";
import type * as Monaco from "monaco-editor";
import {
  celEditorOptions,
  celLanguage,
  configureCelMonaco,
} from "../celMonaco";
import {
  EnumSelector,
  type EnumSelectorOption,
} from "../components/EnumSelector";
import { EmptyState, StatusBanner } from "../components/Primitives";
import { ResultingYaml } from "./ResultingYaml";
import type { AuthorizationDraft } from "./types";

type RuleEffect = "allow" | "deny" | "require";
type AuthzRule = {
  effect: RuleEffect;
  expression: string;
};

const defaultNewRule = 'request.path.startsWith("/v1/")';
const celPlaygroundExpressionKey = "agw.cel.pendingExpression";
const effectOptions: Array<EnumSelectorOption<RuleEffect>> = [
  { value: "allow", label: "Allow", description: "Permit matching requests." },
  { value: "deny", label: "Deny", description: "Reject matching requests." },
  {
    value: "require",
    label: "Require",
    description: "Require this expression to be true.",
  },
];

export function AuthorizationPolicyEditor(props: {
  formId?: string;
  authorization: AuthorizationDraft | null | undefined;
  newRuleExpression?: string;
  saving: boolean;
  onSave: (authorization: AuthorizationDraft) => void;
}) {
  const [rules, setRules] = useState<AuthzRule[]>(() =>
    initialRules(props.authorization),
  );
  const [editingIndex, setEditingIndex] = useState<number | null>(() =>
    rules.length ? 0 : null,
  );
  const [errors, setErrors] = useState<Record<number, string>>({});
  const [summaryError, setSummaryError] = useState<string | null>(null);
  const preview = buildAuthorization(rules);

  function addRule() {
    setRules((current) => [
      ...current,
      {
        effect: "allow",
        expression: props.newRuleExpression ?? defaultNewRule,
      },
    ]);
    setEditingIndex(rules.length);
    setSummaryError(null);
  }

  function updateRule(index: number, value: string) {
    setRules((current) =>
      current.map((rule, ruleIndex) =>
        ruleIndex === index ? { ...rule, expression: value } : rule,
      ),
    );
    clearRuleError(index);
  }

  function updateEffect(index: number, effect: RuleEffect) {
    setRules((current) =>
      current.map((rule, ruleIndex) =>
        ruleIndex === index ? { ...rule, effect } : rule,
      ),
    );
    clearRuleError(index);
  }

  function clearRuleError(index: number) {
    setErrors((current) => {
      if (!current[index]) return current;
      const next = { ...current };
      delete next[index];
      return next;
    });
    setSummaryError(null);
  }

  function removeRule(index: number) {
    setRules((current) =>
      current.filter((_, ruleIndex) => ruleIndex !== index),
    );
    setErrors((current) => remapErrorsAfterDelete(current, index));
    setEditingIndex((current) => {
      if (current === null) return null;
      if (current === index) return null;
      return current > index ? current - 1 : current;
    });
    setSummaryError(null);
  }

  function save() {
    const validationErrors = validateRules(rules);
    setErrors(validationErrors);
    if (Object.keys(validationErrors).length) {
      setEditingIndex(Number(Object.keys(validationErrors)[0]));
      setSummaryError("Fix the highlighted authorization rules before saving.");
      return;
    }
    setSummaryError(null);
    props.onSave(buildAuthorization(rules));
  }

  return (
    <form
      id={props.formId}
      className="policy-editor-stack"
      onSubmit={(event) => {
        event.preventDefault();
        save();
      }}
    >
      <div className="authz-rule-toolbar">
        <div>
          <strong>
            {rules.length} {rules.length === 1 ? "rule" : "rules"}
          </strong>
          <small>
            Each CEL expression is saved under allow, deny, or require.
          </small>
        </div>
        <button className="button" type="button" onClick={addRule}>
          <Plus size={16} />
          Add rule
        </button>
      </div>

      {rules.length === 0 ? (
        <EmptyState
          title="No authorization rules"
          description="Add a CEL expression to start authorizing requests."
          action={
            <button className="button primary" type="button" onClick={addRule}>
              <Plus size={16} />
              Add rule
            </button>
          }
        />
      ) : (
        <div className="authz-rule-list">
          {rules.map((rule, index) => {
            const editing = editingIndex === index;
            return (
              <section
                className={
                  errors[index] ? "authz-rule-card invalid" : "authz-rule-card"
                }
                key={index}
              >
                <div className="authz-rule-header">
                  <div>
                    <div className="authz-rule-title">
                      <strong>Rule {index + 1}</strong>
                      <span className={`badge authz-effect ${rule.effect}`}>
                        {rule.effect}
                      </span>
                    </div>
                    {!editing ? (
                      <code>{rule.expression.trim() || "Empty rule"}</code>
                    ) : null}
                  </div>
                  <div className="row-actions">
                    <div className="authz-effect-select">
                      <EnumSelector
                        ariaLabel={`Rule ${index + 1} effect`}
                        value={rule.effect}
                        options={effectOptions}
                        onChange={(value) => updateEffect(index, value)}
                      />
                    </div>
                    <button
                      className="table-action"
                      type="button"
                      onClick={() => setEditingIndex(editing ? null : index)}
                    >
                      <Pencil size={14} />
                      {editing ? "Done" : "Edit"}
                    </button>
                    <Link
                      className="table-action"
                      to="/cel"
                      onClick={() =>
                        localStorage.setItem(
                          celPlaygroundExpressionKey,
                          rule.expression,
                        )
                      }
                    >
                      <Braces size={14} />
                      Playground
                    </Link>
                    <button
                      className="table-action danger"
                      type="button"
                      onClick={() => removeRule(index)}
                    >
                      <Trash2 size={14} />
                      Delete
                    </button>
                  </div>
                </div>
                {editing ? (
                  <div
                    className={
                      errors[index]
                        ? "editor-wrap mini invalid"
                        : "editor-wrap mini"
                    }
                  >
                    <Editor
                      beforeMount={configureCelMonaco}
                      language={celLanguage}
                      theme={
                        document.documentElement.dataset.theme === "dark"
                          ? "vs-dark"
                          : "light"
                      }
                      value={rule.expression}
                      onChange={(value) => updateRule(index, value ?? "")}
                      onMount={configureMiniEditor}
                      options={{
                        ...celEditorOptions,
                        lineNumbers: "off",
                        wordWrap: "on",
                      }}
                    />
                  </div>
                ) : null}
                {errors[index] ? (
                  <small className="field-error">{errors[index]}</small>
                ) : null}
              </section>
            );
          })}
        </div>
      )}

      <ResultingYaml value={preview} />
      {summaryError ? (
        <StatusBanner state="bad" title="Invalid authorization policy">
          {summaryError}
        </StatusBanner>
      ) : null}
      {!props.formId ? (
        <button
          className="button primary"
          type="submit"
          disabled={props.saving}
        >
          <Save size={16} />
          Apply authorization
        </button>
      ) : null}
    </form>
  );
}

export function pendingCelExpression() {
  const expression = localStorage.getItem(celPlaygroundExpressionKey);
  if (expression !== null) localStorage.removeItem(celPlaygroundExpressionKey);
  return expression;
}

function configureMiniEditor(editor: Monaco.editor.IStandaloneCodeEditor) {
  editor.focus();
}

function initialRules(
  authorization: AuthorizationDraft | null | undefined,
): AuthzRule[] {
  if (!authorization || typeof authorization !== "object") return [];
  const loose = authorization as {
    rules?:
      | Array<string | { allow?: string; deny?: string; require?: string }>
      | { allow?: string[]; deny?: string[]; require?: string[] };
  };
  if (Array.isArray(loose.rules)) {
    return loose.rules.map((rule) => {
      if (typeof rule === "string")
        return { effect: "allow", expression: rule };
      if (rule.deny !== undefined)
        return { effect: "deny", expression: rule.deny };
      if (rule.require !== undefined)
        return { effect: "require", expression: rule.require };
      return { effect: "allow", expression: rule.allow ?? "" };
    });
  }
  const grouped =
    loose.rules && typeof loose.rules === "object" ? loose.rules : {};
  return [
    ...(grouped.deny ?? []).map((expression) => ({
      effect: "deny" as const,
      expression,
    })),
    ...(grouped.allow ?? []).map((expression) => ({
      effect: "allow" as const,
      expression,
    })),
    ...(grouped.require ?? []).map((expression) => ({
      effect: "require" as const,
      expression,
    })),
  ];
}

function buildAuthorization(rules: AuthzRule[]): AuthorizationDraft {
  return {
    rules: rules
      .map((rule) => ({
        effect: rule.effect,
        expression: rule.expression.trim(),
      }))
      .filter((rule) => rule.expression)
      .map((rule) => ({ [rule.effect]: rule.expression })),
  };
}

function validateRules(rules: AuthzRule[]) {
  const errors: Record<number, string> = {};
  if (rules.length === 0) {
    errors[0] = "At least one authorization rule is required.";
    return errors;
  }
  const seen = new Set<string>();
  rules.forEach((rule, index) => {
    const trimmed = rule.expression.trim();
    if (!trimmed) {
      errors[index] = "Rule expression is required.";
      return;
    }
    if (seen.has(trimmed)) {
      errors[index] = "Duplicate rule expression.";
      return;
    }
    seen.add(trimmed);
  });
  return errors;
}

function remapErrorsAfterDelete(
  errors: Record<number, string>,
  deletedIndex: number,
) {
  const next: Record<number, string> = {};
  Object.entries(errors).forEach(([key, value]) => {
    const index = Number(key);
    if (index < deletedIndex) next[index] = value;
    if (index > deletedIndex) next[index - 1] = value;
  });
  return next;
}
