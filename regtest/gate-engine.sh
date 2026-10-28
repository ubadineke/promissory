#!/usr/bin/env bash
# Engine gate on regtest: three installments released by the keeper while the payer app is
# closed, then the payer cancels and the fourth is rejected by the chain as a double-spend.
#
#   regtest/gate-engine.sh <order-id> <blob1> <blob2> <blob3> <blob4>
#
# Needs: regtest/regtest.sh up + automine, and promissory-keeper on 127.0.0.1:8737.
# While waiting it reads only the keeper's public status endpoint: no payer-side process runs
# until the cancel step.
set -euo pipefail
cd "$(dirname "$0")/.."
ORDER=$1; shift
BLOBS=("$@")
KEEPER=http://127.0.0.1:8737
promissory() { ./target/release/promissory --network regtest "$@"; }
state() { curl -sS "$KEEPER/v1/blobs/$1" | jq -r '.state + (if .mined_height then " @" + (.mined_height|tostring) else "" end) + (if .last_error then " (" + .last_error + ")" else "" end)'; }
show() { local line="$(date +%T)"; for i in 0 1 2 3; do line+=" | #$((i+1)) $(state "${BLOBS[$i]}")"; done; echo "$line"; }

echo "payer app closed; watching the keeper"
until [[ "$(state "${BLOBS[2]}")" == mined* ]]; do show; sleep 20; done
show
echo "three installments mined with the payer app closed"

echo "payer reopens the app and cancels"
promissory order cancel "$ORDER" --margin 60 --keep-blobs

until [[ "$(state "${BLOBS[3]}")" =~ ^(cancelled|mined|failed|expired) ]]; do show; sleep 20; done
show
echo "payee balance:"
promissory --dir promissory-data-regtest-payee sync | tail -1
