#!/usr/bin/env bash
# Mede o DER do meetcli nos áudios sintéticos de testdata/out. Uso: avaliar.sh [pasta_modelos]
# Imprime uma linha por (cenário, configuração) e a média por grupo.
set -u
RAIZ="$(cd "$(dirname "$0")/../.." && pwd)"
export RENANMEET_MODELS_DIR="${1:-$RAIZ/.cache/models}" RENANMEET_DATA_DIR="$(mktemp -d)"
CLI="$RAIZ/target/release/meetcli"; PY="$RAIZ/.venv/bin/python"; OUT="$RAIZ/testdata/out"
rodar() { # grupo arquivo k flags...
  local g=$1 f=$2 k=$3; shift 3
  local args=(); [ "$k" != auto ] && args+=(--speakers "$k")
  "$CLI" diarize "$OUT/$f.wav" "${args[@]}" "$@" --saida "$OUT/$f.hip.json" 2>/dev/null || { echo "$g $f ERRO"; return; }
  echo "$g $f $("$PY" -I "$RAIZ/tools/testaudio/der.py" "$OUT/$f.json" "$OUT/$f.hip.json")"
}
{
  for f in online_2; do rodar "online_k_conhecido" $f 2; done
  for f in online_4 online_4_s1 online_4_s2 online_4_s3; do rodar "online_k_conhecido" $f 4; rodar "online_auto" $f auto; done
  for f in sala_4 sala_4_s1 sala_4_s2 sala_4_s3; do
    rodar "canal_unico_k_conhecido" $f 4
    rodar "canal_unico_auto" $f auto
  done
} | "$PY" -I -c '
import sys,json,collections
g=collections.defaultdict(list)
for l in sys.stdin:
    grupo,arq,rest=l.split(" ",2); print(f"{grupo:40s} {arq:12s} {rest.strip()}")
    try: g[grupo].append(json.loads(rest)["der"])
    except Exception: pass
print()
for k,v in g.items(): print(f"MÉDIA {k:40s} DER {sum(v)/len(v):5.1f} %  (n={len(v)}, min {min(v)}, máx {max(v)})")
'
