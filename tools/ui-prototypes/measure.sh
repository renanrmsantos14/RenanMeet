#!/usr/bin/env bash
# Uso: measure.sh <binário>  → RSS e PSS (MB) após 6 s parado, tela :0
"$1" >/dev/null 2>&1 &
pid=$!
sleep 6
if kill -0 $pid 2>/dev/null; then
  rss=$(awk '/VmRSS/{print $2}' /proc/$pid/status)
  pss=$(awk '/^Pss:/{s+=$2} END{print s}' /proc/$pid/smaps_rollup 2>/dev/null)
  echo "$(basename $1): RSS $((rss/1024)) MB  PSS $((pss/1024)) MB"
else
  echo "$(basename $1): morreu antes de 6 s"
fi
kill $pid 2>/dev/null; wait $pid 2>/dev/null
