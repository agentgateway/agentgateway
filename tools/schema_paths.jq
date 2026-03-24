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
  if $description != "" and $note != "" then
    $description + "<br>" + $note
  elif $note != "" then
    $note
  else
    $description
  end;

def simple_type:
  if type == "boolean" then
    ""
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
    ([.oneOf[] | select((union_branch_info | .kind) != "ignore") | simple_type | select(length > 0)] | first) // ""
  elif .anyOf then
    ([.anyOf[] | select((union_branch_info | .kind) != "ignore") | simple_type | select(length > 0)] | first) // ""
  elif .allOf then
    ([.allOf[] | simple_type | select(length > 0)] | first) // ""
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
    ""
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
    [$path, (.value | rendered_description), (.value | simple_type)] as $entry |
    $entry,
    (.value | select(type != "boolean") | schema_paths($path + "."))
  elif (.type // [] | if type == "array" then . else [.] end | contains(["array"])) and .items then
    .items | schema_paths(prefix + "[].")
  elif .properties then
    .properties | to_entries[] |
    (prefix + .key) as $path |
    [$path, (.value | rendered_description), (.value | simple_type)] as $entry |
    $entry,
    (.value | schema_paths($path + "."))
  else
    empty
  end);

[schema_paths("")] | .[]  | ["|`" + .[0] + "`|" + .[2] + "|" + .[1] + "|"] | join(",")
