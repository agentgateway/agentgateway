import type { AdminConfigDump } from "../types";
import { requestJson } from "./base";

export function getConfigDump() {
  return requestJson<AdminConfigDump>("/config_dump");
}
