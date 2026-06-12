# Form Update Workflow

This document describes the workflow for updating forms when the schema changes.

## Workflow

1. **Test the UI** - Use the Traffic page to create or edit configuration items (binds, listeners, routes, backends, etc.)

2. **Look for errors** - When you save, check the toast notifications for validation errors

3. **Copy the error** - Copy the exact error message from the toast notification

4. **Ask Claude to fix it** - Provide Claude with:
   - The error message (paste it)
   - Context: This forms folder
   - Context: `form_gen_context.md` file
   - Request: "Update the form accordingly" or "Fix this validation error"

5. **Test again** - After Claude updates the form, test the UI again to verify the fix

## Example

If you see this error in a toast:

```
"binds[0].listeners[0].routes[0].backends[0]: invalid value: string "asdf", expected string for NamespacedHostname with format namespace/hostname"
```

Then tell Claude:

```
I'm getting this error when editing backends:

"binds[0].listeners[0].routes[0].backends[0]: invalid value: string "asdf", expected string for NamespacedHostname with format namespace/hostname"

Update the backend form to fix this please.
```

Make sure to include:

- `ui/src/pages/Traffic/forms/` folder as context
- `ui/src/pages/Traffic/forms/form_gen_context.md` as context

## Notes

- The forms use RJSF (React JSON Schema Form), not JSON Forms
- Each form has a schema (RJSFSchema), uiSchema (UiSchema), and defaultValues
- The forms should match the structure defined in `schema/config.json`
- Error messages from the server typically indicate what the correct structure should be
