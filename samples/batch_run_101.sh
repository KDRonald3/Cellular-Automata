#!/usr/bin/env bash
# Batch-generate and save all 256 rules × 2 boundary modes
# seed=1, width=101, generations=100, single center cell

BASE="http://127.0.0.1:3000"
DONE=0
SKIP=0
FAIL=0

for BOUNDARY in "ZeroPadded" "Wrap"; do
  for RULE in $(seq 0 255); do
    PAYLOAD=$(printf '{"seed":"1","rule":%d,"generations":100,"width":101,"cell_size":2,"boundary":"%s","align":"Center","fill":"Zero","show_borders":false}' "$RULE" "$BOUNDARY")

    RESP=$(curl -s -X POST "$BASE/api/run" \
      -H "Content-Type: application/json" \
      -d "$PAYLOAD")

    ID=$(echo "$RESP" | grep -o '"id":[0-9]*' | head -1 | grep -o '[0-9]*')

    if [ -z "$ID" ]; then
      echo "FAIL rule=$RULE boundary=$BOUNDARY (no id in response)"
      ((FAIL++))
      continue
    fi

    SAVE=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
      -H "X-Requested-With: batch_run" "$BASE/api/runs/$ID/save")

    if [ "$SAVE" = "200" ]; then
      echo "SAVED rule=$RULE boundary=$BOUNDARY"
      ((DONE++))
    elif [ "$SAVE" = "409" ]; then
      echo "SKIP  rule=$RULE boundary=$BOUNDARY (duplicate)"
      ((SKIP++))
    else
      echo "FAIL  rule=$RULE boundary=$BOUNDARY (save HTTP $SAVE)"
      ((FAIL++))
    fi
  done
done

echo ""
echo "Done=$DONE  Skipped=$SKIP  Failed=$FAIL"
