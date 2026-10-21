#!/bin/sh
while IFS= read -r line; do
  kind=$(printf '%s' "$line" | jq -r .type)
  case "$kind" in
    open) printf '%s\n' '{"protocol":1,"type":"ready"}' ;;
    submit) printf '%s' "$line" | jq -c '{protocol:1,type:"scheduled",attempt_id,uuid}' ;;
    close) exit 0 ;;
    *) exit 2 ;;
  esac
done
