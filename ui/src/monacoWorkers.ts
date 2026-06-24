import { loader } from "@monaco-editor/react";
import * as localMonaco from "monaco-editor/esm/vs/editor/editor.api";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker.js?worker";
import YamlWorker from "monaco-yaml/yaml.worker.js?worker";
import "monaco-editor/esm/vs/basic-languages/yaml/yaml.contribution.js";
import "monaco-editor/esm/vs/editor/contrib/hover/browser/hoverContribution.js";
import "monaco-editor/esm/vs/editor/contrib/suggest/browser/suggestController.js";

let workersConfigured = false;

export function configureConfigMonacoWorkers() {
  if (workersConfigured) return;
  workersConfigured = true;

  globalThis.MonacoEnvironment = {
    getWorker(_moduleId: string, label: string) {
      if (label === "yaml") {
        return new YamlWorker();
      }
      return new EditorWorker();
    },
  };
}

loader.config({ monaco: localMonaco });
configureConfigMonacoWorkers();

declare global {
  // Monaco reads this global directly when resolving web workers.
  // eslint-disable-next-line no-var
  var MonacoEnvironment:
    | {
        getWorker(moduleId: string, label: string): Worker;
      }
    | undefined;
}
