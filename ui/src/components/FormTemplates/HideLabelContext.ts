import { createContext } from "react";

/**
 * Context used by CollapsibleObjectFieldTemplate to tell FieldTemplate
 * which field IDs should suppress their label because the parent section
 * title already conveys the same information.
 */
export const HideLabelContext = createContext<Set<string>>(new Set());
