import { requestJson } from "./base";

export async function evaluateCel(expression: string, data: unknown) {
  return requestJson<{ result: unknown; error: string | null }>("/cel", {
    method: "POST",
    body: JSON.stringify({ expression, data }),
  });
}
