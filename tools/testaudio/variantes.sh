#!/usr/bin/env bash
# Rode da raiz do repo. Ex.: tools/testaudio/variantes.sh "rótulo" --sem-refinar
# uso: variantes.sh <rótulo> <flags...>   → DER médio nos 4 sala_4
r=$1; shift; soma=0; linha=""
for f in sala_4 sala_4_s1 sala_4_s2 sala_4_s3; do
  ./target/release/meetcli diarize testdata/out/$f.wav --speakers 4 "$@" --saida testdata/out/$f.v.json 2>/dev/null
  d=$(.venv/bin/python -I tools/testaudio/der.py testdata/out/$f.json testdata/out/$f.v.json | sed 's/.*"der": \([0-9.]*\),.*/\1/'); linha="$linha $d"; soma=$(echo "$soma + $d" | bc -l)
done
echo "$r:$linha | média $(echo "scale=1; $soma/4" | bc -l)"
