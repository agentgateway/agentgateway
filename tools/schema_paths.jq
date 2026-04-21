def arrayify:
  if type == "array" then . else [.] end;

def join_or(items):
  if (items | length) == 0 then
    ""
  elif (items | length) == 1 then
    items[0]
  elif (items | length) == 2 then
    items[0] + " or " + items[1]
  else
    (items[0:-1] | join(", ")) + ", or " + items[-1]
  end;

def unique_preserve_order(items):
  reduce items[] as $item ([]; if index($item) == null then . + [$item] else . end);

def enum_info:
  if .const? != null then
    {"kind": "enum", "values": [.const]}
  elif .enum? then
    {"kind": "enum", "values": .enum}
  elif .type? then
    (.type | arrayify | map(select(. != "null"))) as $types |
    if ($types | length) == 0 then
      {"kind": "null", "values": []}
    elif ($types | length) == 1 and $types[0] == "array" and .items then
      (.items | enum_info) as $items |
      if $items.kind == "enum" then
        {"kind": "array", "values": $items.values}
      else
        {"kind": "other", "values": []}
      end
    else
      {"kind": "other", "values": []}
    end
  elif .oneOf then
    ([.oneOf[] | enum_info]) as $info |
    ($info | map(select(.kind != "null"))) as $nonnull |
    if ($nonnull | length) > 0 and ($nonnull | all(.kind == "enum")) then
      {"kind": "enum", "values": unique_preserve_order($nonnull | map(.values[]) )}
    elif ($nonnull | length) > 0 and ($nonnull | all(.kind == "array")) then
      {"kind": "array", "values": unique_preserve_order($nonnull | map(.values[]) )}
    else
      {"kind": "other", "values": []}
    end
  elif .anyOf then
    ([.anyOf[] | enum_info]) as $info |
    ($info | map(select(.kind != "null"))) as $nonnull |
    if ($nonnull | length) > 0 and ($nonnull | all(.kind == "enum")) then
      {"kind": "enum", "values": unique_preserve_order($nonnull | map(.values[]) )}
    elif ($nonnull | length) > 0 and ($nonnull | all(.kind == "array")) then
      {"kind": "array", "values": unique_preserve_order($nonnull | map(.values[]) )}
    else
      {"kind": "other", "values": []}
    end
  elif .allOf then
    ([.allOf[] | enum_info]) as $info |
    ($info | map(select(.kind != "null"))) as $nonnull |
    if ($nonnull | length) > 0 and ($nonnull | all(.kind == "enum")) then
      {"kind": "enum", "values": unique_preserve_order($nonnull | map(.values[]) )}
    elif ($nonnull | length) > 0 and ($nonnull | all(.kind == "array")) then
      {"kind": "array", "values": unique_preserve_order($nonnull | map(.values[]) )}
    else
      {"kind": "other", "values": []}
    end
  else
    {"kind": "other", "values": []}
  end;

def formatted_enum_values(values):
  values | map("`" + tostring + "`") | join(", ");

def enum_note:
  (enum_info) as $enum |
  if ($enum.kind == "enum" or $enum.kind == "array") and ($enum.values | length) > 0 then
    "Possible values: " + formatted_enum_values($enum.values) + "."
  else
    ""
  end;

def union_branch_info:
  (.type? | arrayify | map(select(. != "null"))) as $types |
  if ($types | length) == 0 then
    {"kind": "null"}
  elif (.enum? == ["invalid"]) and ($types | length) == 1 and $types[0] == "string" then
    {"kind": "ignore"}
  elif ($types | length) == 1 and $types[0] == "object" and .properties and ((.properties | keys_unsorted | length) == 1) then
    {"kind": "key", "key": (.properties | keys_unsorted[0])}
  else
    {"kind": "other"}
  end;

def simple_union_note:
  if type != "object" then
    ""
  else
    (if .oneOf then .oneOf elif .anyOf then .anyOf else [] end) as $branches |
    ($branches | map(union_branch_info)) as $info |
    ($info | map(select(.kind == "key") | .key)) as $keys |
    if ($keys | length) >= 2 and ($info | map(select(.kind == "other")) | length) == 0 then
      if .oneOf then
        "Exactly one of " + join_or($keys) + " may be set."
      else
        "One or more of " + join_or($keys) + " may be set."
      end
    else
      ""
    end
  end;

def rendered_description:
  (.description? // "" | sub("\n"; "<br>"; "g")) as $description |
  (simple_union_note) as $note |
  (enum_note) as $enum_note |
  ($description | test("Accepted values:|Possible values:")) as $has_enum_note |
  if $description != "" and $note != "" and $enum_note != "" and ($has_enum_note | not) then
    $description + "<br>" + $note + "<br>" + $enum_note
  elif $description != "" and $note != "" then
    $description + "<br>" + $note
  elif $description != "" and $enum_note != "" and ($has_enum_note | not) then
    $description + "<br>" + $enum_note
  elif $note != "" and $enum_note != "" then
    $note + "<br>" + $enum_note
  elif $note != "" then
    $note
  elif $enum_note != "" and ($has_enum_note | not) then
    $enum_note
  else
    $description
  end;

def simple_type:
  if type == "boolean" then
    "any"
  elif (enum_info | .kind) == "enum" then
    "enum"
  elif (enum_info | .kind) == "array" then
    "[]enum"
  elif .type? then
    (.type | arrayify | map(select(. != "null"))) as $types |
    if ($types | length) == 1 and $types[0] == "array" then
      if .items then
        (.items | simple_type) as $item_type |
        if $item_type == "string" or $item_type == "object" or $item_type == "integer" or $item_type == "number" or $item_type == "boolean" then
          "[]" + $item_type
        else
          "array"
        end
      else
        "array"
      end
    else
      ($types | first) // ""
    end
  elif .oneOf then
    (([.oneOf[] | select((union_branch_info | .kind) != "ignore") | select(((enum_info | .kind) != "enum") and ((enum_info | .kind) != "array")) | simple_type | select(length > 0)] | first) //
    ([.oneOf[] | select((union_branch_info | .kind) != "ignore") | simple_type | select(length > 0)] | first)) // ""
  elif .anyOf then
    (([.anyOf[] | select((union_branch_info | .kind) != "ignore") | select(((enum_info | .kind) != "enum") and ((enum_info | .kind) != "array")) | simple_type | select(length > 0)] | first) //
    ([.anyOf[] | select((union_branch_info | .kind) != "ignore") | simple_type | select(length > 0)] | first)) // ""
  elif .allOf then
    (([.allOf[] | select(((enum_info | .kind) != "enum") and ((enum_info | .kind) != "array")) | simple_type | select(length > 0)] | first) //
    ([.allOf[] | simple_type | select(length > 0)] | first)) // ""
  elif .properties then
    "object"
  elif .items then
    (.items | simple_type) as $item_type |
    if $item_type == "string" or $item_type == "object" or $item_type == "integer" or $item_type == "number" or $item_type == "boolean" then
      "[]" + $item_type
    else
      "array"
    end
  else
    "any"
  end;

def schema_paths(prefix):
  (if .oneOf then
    .oneOf[] | schema_paths(prefix)
  elif .anyOf then
    .anyOf[] | schema_paths(prefix)
  elif .allOf then
    .allOf[] | schema_paths(prefix)
  else
    empty
  end),

  (if (.type // [] | if type == "array" then . else [.] end | contains(["object"])) and .properties then
    .properties | to_entries[] |
    (prefix + .key) as $path |
    [$path, ((.value | rendered_description) // ""), ((.value | simple_type) // "")] as $entry |
    $entry,
    (.value | select(type != "boolean") | schema_paths($path + "."))
  elif (.type // [] | if type == "array" then . else [.] end | contains(["array"])) and .items then
    .items | schema_paths(prefix + "[].")
  elif .properties then
    .properties | to_entries[] |
    (prefix + .key) as $path |
    [$path, ((.value | rendered_description) // ""), ((.value | simple_type) // "")] as $entry |
    $entry,
    (.value | schema_paths($path + "."))
  else
    empty
  end);

[schema_paths("")] | .[]  | ["|`" + .[0] + "`|" + .[2] + "|" + .[1] + "|"] | join(",")
