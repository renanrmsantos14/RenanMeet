#!/usr/bin/env bash
# Dados de demonstração para ver o app sem chave nem reunião real: Groq falsa + áudio sintético.
# Uso: demo.sh <pasta-de-dados>   (depois: RENANMEET_DATA_DIR=<pasta> target/release/renanmeet)
set -euo pipefail
RAIZ="$(cd "$(dirname "$0")/../.." && pwd)"; D="${1:?uso: demo.sh <pasta-de-dados>}"
WAV="$RAIZ/testdata/out/online_2.wav"; VERDADE="$RAIZ/testdata/out/online_2.json"
CLI="$RAIZ/target/release/meetcli"; T="$(mktemp -d)"; trap 'kill $MOCK 2>/dev/null; rm -rf "$T"' EXIT
rm -rf "$D"; mkdir -p "$D"
export RENANMEET_MODELS_DIR="${RENANMEET_MODELS_DIR:-$RAIZ/.cache/models}" RENANMEET_DATA_DIR="$D" RENANMEET_GROQ_KEY=chave-falsa
python3 -I "$RAIZ/tools/testaudio/groq_mock.py" "$VERDADE" > "$T/porta" & MOCK=$!
for _ in $(seq 50); do [ -s "$T/porta" ] && break; sleep 0.1; done
export RENANMEET_GROQ_BASE="http://127.0.0.1:$(cat "$T/porta")"
for t in "Alinhamento do contrato" "Planejamento da semana"; do
  "$CLI" import "$WAV" --titulo "$t" >/dev/null; ID=$("$CLI" list | head -1 | cut -f1)
  "$CLI" process "$ID" >/dev/null 2>&1; sleep 1.1
done
ID=$("$CLI" list | tail -1 | cut -f1)
"$CLI" name "$ID" 1 Renan --guardar >/dev/null; "$CLI" name "$ID" 2 Ana --guardar >/dev/null
echo "demo pronta em $D"
