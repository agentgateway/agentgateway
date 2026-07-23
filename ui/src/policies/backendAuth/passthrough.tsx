import type { SchemaHelp } from "../../schemaHelp";
import {
  authorizationLocationFrom,
  authorizationLocationToValue,
  CredentialLocationSetting,
  type AuthorizationLocationDraft,
} from "../AuthorizationLocation";
import { cleanEmpty } from "../policyUtils";

// -- Passthrough draft --

export type PassthroughDraft = { location: AuthorizationLocationDraft };

export function emptyPassthroughDraft(): PassthroughDraft {
  return { location: authorizationLocationFrom(undefined) };
}

export function passthroughDraftFromValue(
  value: Record<string, unknown>,
): PassthroughDraft {
  return { location: authorizationLocationFrom(value.location) };
}

export function passthroughDraftToValue(draft: PassthroughDraft): unknown {
  return (
    cleanEmpty({
      location: authorizationLocationToValue(draft.location),
    }) ?? {}
  );
}

export function PassthroughFields(props: {
  value: PassthroughDraft;
  help: SchemaHelp;
  onChange: (next: PassthroughDraft) => void;
}) {
  return (
    <CredentialLocationSetting
      value={props.value.location}
      help={props.help}
      defaultDescription="Default: Authorization: Bearer token"
      description="Override where the validated credential is sent."
      allowExpression={false}
      onChange={(location) => props.onChange({ ...props.value, location })}
    />
  );
}
