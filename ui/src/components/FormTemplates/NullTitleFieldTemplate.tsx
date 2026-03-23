/**
 * A TitleFieldTemplate that renders nothing.
 *
 * RJSF renders TitleField from SchemaField *before* calling ObjectFieldTemplate,
 * which would produce a plain <label> that duplicates the bold heading our
 * CollapsibleObjectFieldTemplate already renders via <Title level={5}>.
 * By suppressing the default TitleField here we get exactly one bold title.
 */
export function NullTitleFieldTemplate() {
  return null;
}
