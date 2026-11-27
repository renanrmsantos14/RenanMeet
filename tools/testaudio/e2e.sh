#!/usr/bin/env bash
# Ponta a ponta sem rede e sem chave: Groq falsa (groq_mock.py) + Sherpa real + wiki de mentira.
# Uso: e2e.sh [audio.wav verdade.json]  (padrão: testdata/out/online_2). Sai com erro se algo falhar.
set -euo pipefail
RAIZ="$(cd "$(dirname "$0")/../.." && pwd)"
WAV="${1:-$RAIZ/testdata/out/online_2.wav}"; VERDADE="${2:-$RAIZ/testdata/out/online_2.json}"
CLI="$RAIZ/target/release/meetcli"; T="$(mktemp -d)"; trap 'kill $MOCK 2>/dev/null; rm -rf "$T"' EXIT
export RENANMEET_MODELS_DIR="${RENANMEET_MODELS_DIR:-$RAIZ/.cache/models}" RENANMEET_DATA_DIR="$T/data" RENANMEET_GROQ_KEY=chave-falsa
falha() { echo "FALHOU: $*" >&2; exit 1; }
# wiki de mentira com remoto local
git init -q --bare "$T/remoto.git"; git clone -q "$T/remoto.git" "$T/wiki" 2>/dev/null; mkdir "$T/wiki/inbox"; touch "$T/wiki/inbox/.keep"
git -C "$T/wiki" add -A; git -C "$T/wiki" -c user.email=t@t -c user.name=t commit -qm init; git -C "$T/wiki" push -q origin HEAD
export RENANMEET_WIKI_DIR="$T/wiki"
python3 -I "$RAIZ/tools/testaudio/groq_mock.py" "$VERDADE" > "$T/porta" & MOCK=$!
for _ in $(seq 50); do [ -s "$T/porta" ] && break; sleep 0.1; done; [ -s "$T/porta" ] || falha "a Groq falsa não subiu"
export RENANMEET_GROQ_BASE="http://127.0.0.1:$(cat "$T/porta")"

"$CLI" key check >/dev/null || falha "chave"
"$CLI" import "$WAV" --titulo "Reuniao A" >/dev/null; ID=$("$CLI" list | cut -f1)
/usr/bin/time -f "tempo %e s · pico %M KB" "$CLI" process "$ID" 2>&1 | tail -1
[ "$("$CLI" list | cut -f2)" = done ] || falha "reunião não terminou em done"
MD=$("$CLI" export "$ID")
# cada trecho "[Nome] frase" deve cair no mesmo falante: verdade ↔ cluster
python3 -I - "$MD" <<'P' || falha "atribuição de falante inconsistente"
import re, sys, collections
m = collections.defaultdict(set)
for l in sys.argv[1].splitlines():
    r = re.match(r"\[\d+:\d+\] (.+?): \[(.+?)\] frase", l)
    if r: m[r[2]].add(r[1])
print("verdade → falante:", {k: sorted(v) for k, v in m.items()})
assert m and all(len(v) == 1 for v in m.values()) and len({next(iter(v)) for v in m.values()}) == len(m)
P
echo "$MD" | grep -q "## Decisões" || falha "ata sem decisões"
# nomeia e guarda; a 2ª reunião precisa reconhecer as vozes sozinha
"$CLI" name "$ID" 1 Renan --guardar; "$CLI" name "$ID" 2 Ana --guardar
"$CLI" wiki "$ID" >/dev/null; git -C "$T/remoto.git" log --oneline | grep -q "Inbox: reunião Reuniao A" || falha "wiki não chegou ao remoto"
sleep 1.1; "$CLI" import "$WAV" --titulo "Reuniao B" >/dev/null; ID2=$("$CLI" list | cut -f1 | head -1)
"$CLI" process "$ID2" 2>/dev/null >/dev/null
LINHA=$("$CLI" export "$ID2" | sed -n 3p); echo "$LINHA" | grep -q "Renan" && echo "$LINHA" | grep -q "Ana" || falha "voz conhecida não reconhecida"
"$CLI" search frase | grep -q "Reuniao" || falha "busca"
echo "OK: importação, processamento, atribuição, ata, wiki, voz conhecida e busca"
