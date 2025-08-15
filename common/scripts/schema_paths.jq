def schema_paths(prefix):
  if .oneOf then
    .oneOf[] | schema_paths(prefix + "(1)")
  elif .anyOf then
    .anyOf[] | schema_paths(prefix + "(any)")
  elif .allOf then
    .allOf[] | schema_paths(prefix + "(all)")
  elif (.type // [] | if type == "array" then . else [.] end | contains(["object"])) and .properties then
    .properties | to_entries[] |
    (prefix + .key) as $path |
    [$path, (.value.description? // "")] as $entry |
    $entry,
    (.value | select(type != "boolean") | schema_paths($path + "."))
  elif (.type // [] | if type == "array" then . else [.] end | contains(["array"])) and .items then
    .items | schema_paths(prefix + "[].")
  elif .properties then
    .properties | to_entries[] |
    (prefix + .key) as $path |
    [$path, (.value.description? // "")] as $entry |
    $entry,
    (.value | schema_paths($path + "."))
  else
    empty
  end;

[schema_paths("")] | .[]  | ["|`" + .[0] + "`|" + .[1] + "|"] | join(",")