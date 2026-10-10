# Input is a schema whose types live in $defs. References are expanded as they are
# walked, with recursive types left as references.
#
# Named arguments:
#   shared: {"<definition>": "<Name>"}. Fields of these types are documented once,
#           so a field of a shared type is a single row linking to `<Name>`.
#   root:   a definition in `shared` to document, instead of the whole schema.
."$defs" as $DEFS |
($ARGS.named.shared // {}) as $SHARED |
($ARGS.named.root // "") as $ROOT |
# A shared type's own section expands everything rather than linking elsewhere.
(if $ROOT == "" then $SHARED else {} end) as $CUT |

# deref($stack) emits [node, stack] with any top-level $ref expanded.
# $stack holds the definitions being expanded, so a recursive type stays a reference.
def deref($stack):
  if type == "object" and has("$ref") then
    (."$ref" | split("/")[-1]) as $name |
    if any($stack[]; . == $name) then
      [., $stack]
    else
      ($DEFS[$name] + del(.["$ref"])) | deref($stack + [$name])
    end
  else
    [., $stack]
  end;

# The `shared` name of a node that is (optionally) a shared type, or null.
def shared_name:
  if type == "object" then
    ([(.anyOf // .oneOf // [.])[] | select(.type? != "null")]) as $branches |
    if ($branches | length) == 1 and ($branches[0] | has("$ref")) then
      $CUT[$branches[0]."$ref" | split("/")[-1]]
    else
      null
    end
  else
    null
  end;

def shared_link($name):
  "[`<" + $name + ">`](#" + ($name | ascii_downcase) + ")";

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

def enum_info($s):
  deref($s) as [$node, $s] | $node |
  if type != "object" then
    {"kind": "other", "values": []}
  elif .const? != null then
    {"kind": "enum", "values": [.const]}
  elif .enum? then
    {"kind": "enum", "values": .enum}
  elif .type? then
    (.type | arrayify | map(select(. != "null"))) as $types |
    if ($types | length) == 0 then
      {"kind": "null", "values": []}
    elif ($types | length) == 1 and $types[0] == "array" and .items then
      (.items | enum_info($s)) as $items |
      if $items.kind == "enum" then
        {"kind": "array", "values": $items.values}
      else
        {"kind": "other", "values": []}
      end
    else
      {"kind": "other", "values": []}
    end
  elif .oneOf then
    ([.oneOf[] | enum_info($s)]) as $info |
    ($info | map(select(.kind != "null"))) as $nonnull |
    if ($nonnull | length) > 0 and ($nonnull | all(.kind == "enum")) then
      {"kind": "enum", "values": unique_preserve_order($nonnull | map(.values[]) )}
    elif ($nonnull | length) > 0 and ($nonnull | all(.kind == "array")) then
      {"kind": "array", "values": unique_preserve_order($nonnull | map(.values[]) )}
    else
      {"kind": "other", "values": []}
    end
  elif .anyOf then
    ([.anyOf[] | enum_info($s)]) as $info |
    ($info | map(select(.kind != "null"))) as $nonnull |
    if ($nonnull | length) > 0 and ($nonnull | all(.kind == "enum")) then
      {"kind": "enum", "values": unique_preserve_order($nonnull | map(.values[]) )}
    elif ($nonnull | length) > 0 and ($nonnull | all(.kind == "array")) then
      {"kind": "array", "values": unique_preserve_order($nonnull | map(.values[]) )}
    else
      {"kind": "other", "values": []}
    end
  elif .allOf then
    ([.allOf[] | enum_info($s)]) as $info |
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

def enum_note($s):
  (enum_info($s)) as $enum |
  if ($enum.kind == "enum" or $enum.kind == "array") and ($enum.values | length) > 0 then
    "Possible values: " + formatted_enum_values($enum.values) + "."
  else
    ""
  end;

def union_branch_info($s):
  deref($s)[0] |
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

def simple_union_note($s):
  if type != "object" then
    ""
  else
    (if .oneOf then .oneOf elif .anyOf then .anyOf else [] end) as $branches |
    ($branches | map(union_branch_info($s))) as $info |
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

def rendered_description($s):
  deref($s) as [$node, $s] | $node |
  (.description? // "" | sub("\n"; "<br>"; "g")) as $description |
  (simple_union_note($s)) as $note |
  (enum_note($s)) as $enum_note |
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

def simple_type($s):
  (shared_name) as $shared |
  deref($s) as [$node, $s] | $node |
  if $shared != null then
    shared_link($shared)
  elif type == "boolean" then
    "any"
  elif (enum_info($s) | .kind) == "enum" then
    "enum"
  elif (enum_info($s) | .kind) == "array" then
    "[]enum"
  elif .type? then
    (.type | arrayify | map(select(. != "null"))) as $types |
    if ($types | length) == 1 and $types[0] == "array" then
      if .items then
        (.items | simple_type($s)) as $item_type |
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
    (([.oneOf[] | select((union_branch_info($s) | .kind) != "ignore") | select(((enum_info($s) | .kind) != "enum") and ((enum_info($s) | .kind) != "array")) | simple_type($s) | select(length > 0)] | first) //
    ([.oneOf[] | select((union_branch_info($s) | .kind) != "ignore") | simple_type($s) | select(length > 0)] | first)) // ""
  elif .anyOf then
    (([.anyOf[] | select((union_branch_info($s) | .kind) != "ignore") | select(((enum_info($s) | .kind) != "enum") and ((enum_info($s) | .kind) != "array")) | simple_type($s) | select(length > 0)] | first) //
    ([.anyOf[] | select((union_branch_info($s) | .kind) != "ignore") | simple_type($s) | select(length > 0)] | first)) // ""
  elif .allOf then
    (([.allOf[] | select(((enum_info($s) | .kind) != "enum") and ((enum_info($s) | .kind) != "array")) | simple_type($s) | select(length > 0)] | first) //
    ([.allOf[] | simple_type($s) | select(length > 0)] | first)) // ""
  elif .properties then
    "object"
  elif .items then
    (.items | simple_type($s)) as $item_type |
    if $item_type == "string" or $item_type == "object" or $item_type == "integer" or $item_type == "number" or $item_type == "boolean" then
      "[]" + $item_type
    else
      "array"
    end
  else
    "any"
  end;

def preserve_union_branch_description($s):
  (.description? // "") as $branch_description |
  if $branch_description != "" and .properties and ((.properties | keys_unsorted | length) == 1) then
    .properties |= with_entries(
      (.value | deref($s)[0] | .description? // "") as $property_description |
      .value.description =
        if $property_description == "" then
          $branch_description
        elif $property_description == $branch_description then
          $property_description
        else
          $branch_description + "\n" + $property_description
        end
    )
  else
    .
  end;

def schema_paths(prefix; $s):
  deref($s) as [$node, $s] | $node |
  # An Option<T> is an anyOf of T and null; it is not a union, so its branch is walked as-is.
  ([(.oneOf // .anyOf // [])[] | select(.type? != "null")] | length > 1) as $union |
  (if .oneOf then
    .oneOf[] | deref($s) as [$branch, $bs] | $branch | (if $union then preserve_union_branch_description($bs) else . end) | schema_paths(prefix; $bs)
  elif .anyOf then
    .anyOf[] | deref($s) as [$branch, $bs] | $branch | (if $union then preserve_union_branch_description($bs) else . end) | schema_paths(prefix; $bs)
  elif .allOf then
    .allOf[] | schema_paths(prefix; $s)
  else
    empty
  end),

  (if (.type // [] | if type == "array" then . else [.] end | contains(["object"])) and .properties then
    .properties | to_entries[] |
    (prefix + .key) as $path |
    [$path, ((.value | rendered_description($s)) // ""), ((.value | simple_type($s)) // "")] as $entry |
    $entry,
    (.value | select(type != "boolean" and shared_name == null) | schema_paths($path + "."; $s))
  elif (.type // [] | if type == "array" then . else [.] end | contains(["array"])) and .items then
    .items | select(type != "boolean") | schema_paths(prefix + "[]."; $s)
  elif (.type // [] | if type == "array" then . else [.] end | contains(["object"])) and (.additionalProperties | type == "object") then
    .additionalProperties | schema_paths(prefix + "*."; $s)
  elif .properties then
    .properties | to_entries[] |
    (prefix + .key) as $path |
    [$path, ((.value | rendered_description($s)) // ""), ((.value | simple_type($s)) // "")] as $entry |
    $entry,
    (.value | select(shared_name == null) | schema_paths($path + "."; $s))
  else
    empty
  end),

  (if (.type // [] | if type == "array" then . else [.] end | contains(["object"])) and .additionalProperties == true then
    [prefix + "*", "", "any"]
  else
    empty
  end);

(if $ROOT == "" then
  [schema_paths(""; [])]
else
  $SHARED[$ROOT] as $name |
  [{"$ref": ("#/$defs/" + $ROOT)} | schema_paths("<" + $name + ">."; [])]
end) | .[]  | ["|`" + .[0] + "`|" + .[2] + "|" + .[1] + "|"] | join(",")
