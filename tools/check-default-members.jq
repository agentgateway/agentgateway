. as $metadata
| ["xtask"] as $allowed_omissions
| [
    (.workspace_members - .workspace_default_members)[] as $id
    | $metadata.packages[]
    | select(.id == $id)
    | .name
    | select(. as $name | $allowed_omissions | index($name) | not)
  ] as $missing
| if ($missing | length) == 0 then
    true
  else
    error("workspace crates missing from default-members: " + ($missing | join(", ")))
  end
