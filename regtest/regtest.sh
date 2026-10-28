#!/usr/bin/env bash
# Local regtest chain: Zebra (blocks on demand via `generate`) + lightwalletd.
#
#   regtest/regtest.sh up <miner-ua>   start (or restart) with coinbase paid to <miner-ua>
#   regtest/regtest.sh mine <n>        mine n blocks
#   regtest/regtest.sh height          print the tip height
#   regtest/regtest.sh automine [secs] mine a block every secs (default 15) until interrupted
#   regtest/regtest.sh logs [zebra|lwd]
#   regtest/regtest.sh down            stop containers (chain state kept)
#   regtest/regtest.sh wipe            stop and delete chain state
#
# REGTEST_PROFILE=nu7 runs a second, independent chain for the NU7 gate: NU7 at height 300
# (override with NU7_AT), its own containers, volume and network, Zebra RPC on 18332 and
# lightwalletd on 9167.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
NET=so-regtest
ZEBRA=so-zebra
LWD=so-lwd
STATE=so-zebra-state
RPC_PORT=18232
LWD_PORT=9067
NU7_HEIGHT=5000
CONF="$HERE/.zebrad.toml"
case "${REGTEST_PROFILE:-}" in
  "") ;;
  nu7)
    NET=so-regtest-nu7 ZEBRA=so-zebra-nu7 LWD=so-lwd-nu7 STATE=so-zebra-nu7-state
    RPC_PORT=18332 LWD_PORT=9167 NU7_HEIGHT="${NU7_AT:-300}" CONF="$HERE/.zebrad-nu7.toml"
    ;;
  *) echo "unknown REGTEST_PROFILE ${REGTEST_PROFILE}" >&2; exit 1 ;;
esac
RPC=http://127.0.0.1:$RPC_PORT
# Zebra's built-in regtest default, used until our wallet has an address.
DEFAULT_MINER=uregtest1efxggx6lduhm2fx5lnrhxv7h7kpztlpa3ahf3n4w0q0zj5epj4av9xjq6ljsja3xk8z7rzd067kc7mgpy9448rdfzpfjz5gq389zdmpgnk6rp4ykk0xk6cmqw6zqcrnmsuaxv3yzsvcwsd4gagtalh0uzrdvy03nhmltjz2eu0232qlcs0zvxuqyut73yucd9gy5jaudnyt7yqhgpqv

rpc() {
  curl -sS --fail -m 120 -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":${2:-[]}}" "$RPC"
}

wait_rpc() {
  for _ in $(seq 1 60); do
    rpc getblockcount >/dev/null 2>&1 && return 0
    sleep 1
  done
  echo "zebra RPC did not come up; see: $0 logs zebra" >&2
  return 1
}

case "${1:-}" in
  up)
    miner="${2:-$DEFAULT_MINER}"
    docker network inspect $NET >/dev/null 2>&1 || docker network create $NET >/dev/null
    docker volume inspect $STATE >/dev/null 2>&1 || docker volume create $STATE >/dev/null
    docker rm -f $ZEBRA $LWD >/dev/null 2>&1 || true
    sed -e "s|__MINER_ADDRESS__|$miner|" -e "s|__NU7_HEIGHT__|$NU7_HEIGHT|" \
      "$HERE/zebrad.toml.tmpl" > "$CONF"
    docker run -d --name $ZEBRA --network $NET -p 127.0.0.1:$RPC_PORT:18232 \
      -v "$CONF:/etc/zebrad.toml:ro" -v $STATE:/home/zebra/.cache/zebra \
      --entrypoint zebrad zfnd/zebra:latest -c /etc/zebrad.toml start >/dev/null
    wait_rpc
    # lightwalletd needs at least one block past genesis to start.
    [ "$(rpc getblockcount | sed 's/.*"result":\([0-9]*\).*/\1/')" -ge 1 ] || rpc generate '[1]' >/dev/null
    docker run -d --name $LWD --network $NET --platform linux/amd64 -p 127.0.0.1:$LWD_PORT:9067 \
      --entrypoint lightwalletd electriccoinco/lightwalletd:latest \
      --no-tls-very-insecure --grpc-bind-addr 0.0.0.0:9067 --data-dir /tmp/lwd \
      --log-file /dev/stdout --rpchost $ZEBRA --rpcport 18232 \
      --rpcuser so --rpcpassword so >/dev/null
    echo "zebra RPC $RPC, lightwalletd http://127.0.0.1:$LWD_PORT, miner $miner"
    ;;
  mine)
    rpc generate "[${2:-1}]" >/dev/null
    rpc getblockcount | sed 's/.*"result":\([0-9]*\).*/\1/'
    ;;
  height)
    rpc getblockcount | sed 's/.*"result":\([0-9]*\).*/\1/'
    ;;
  automine)
    # Mine one block every N seconds (default 15) until interrupted, like a live chain.
    every="${2:-15}"
    while true; do
      rpc generate '[1]' >/dev/null && echo "$(date +%T) mined $(rpc getblockcount | sed 's/.*"result":\([0-9]*\).*/\1/')"
      sleep "$every"
    done
    ;;
  logs)
    docker logs --tail 50 "$([ "${2:-zebra}" = lwd ] && echo $LWD || echo $ZEBRA)"
    ;;
  down)
    docker rm -f $ZEBRA $LWD >/dev/null 2>&1 || true
    ;;
  wipe)
    docker rm -f $ZEBRA $LWD >/dev/null 2>&1 || true
    docker volume rm $STATE >/dev/null 2>&1 || true
    ;;
  *)
    sed -n '2,15p' "$0"
    exit 1
    ;;
esac
