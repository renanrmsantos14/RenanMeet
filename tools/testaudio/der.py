#!/usr/bin/env python3
"""DER (erro de diarização) com verdade em JSON. Uso: der.py verdade.json hipotese [--collar 0.25]
Hipótese: JSON [{start,end,speaker}] ou linhas "ini -- fim speaker_NN" (saída do sherpa).
Quadros de 10 ms; mapeamento ótimo rótulo→falante por permutação; colar ao redor de cada
fronteira da verdade (padrão 0,25 s). Fala sobreposta conta: perdida se o sistema só emite um rótulo."""
import itertools, json, re, sys
import numpy as np

def carrega(p):
    t = open(p, encoding="utf-8").read().strip()
    if t.startswith("["): return [(d["start"], d["end"], d["speaker"]) for d in json.loads(t)]
    out = []
    for l in t.splitlines():
        m = re.match(r"\s*([\d.]+)\s*--\s*([\d.]+)\s+(\S+)", l)
        if m: out.append((float(m[1]), float(m[2]), m[3]))
    return out

def der(ref, hyp, collar=0.25, step=0.01):
    fim = max(e for _, e, _ in ref + hyp) + 1; n = int(fim / step)
    rs = sorted({s for *_, s in ref}); hs = sorted({s for *_, s in hyp})
    R = np.zeros((len(rs), n), bool); H = np.zeros((len(hs), n), bool); ok = np.ones(n, bool)
    for a, b, s in ref:
        R[rs.index(s), int(a / step):int(b / step)] = True
        for t in (a, b): ok[max(int((t - collar) / step), 0):int((t + collar) / step)] = False
    for a, b, s in hyp: H[hs.index(s), int(a / step):int(b / step)] = True
    R, H = R[:, ok], H[:, ok]; nref, nhyp = R.sum(0), H.sum(0); total = nref.sum()
    # melhor mapeamento hip→ref (permutação; ≤ 6 rótulos)
    melhor = 0
    ks = range(len(hs)); 
    for perm in itertools.permutations(range(len(rs)), min(len(hs), len(rs))):
        c = sum((H[i] & R[perm[i]]).sum() for i in range(len(perm)))
        melhor = max(melhor, c)
    miss = np.maximum(nref - nhyp, 0).sum(); fa = np.maximum(nhyp - nref, 0).sum()
    conf = np.minimum(nref, nhyp).sum() - melhor
    return dict(der=round(float((miss + fa + conf) / total) * 100, 1), miss=round(float(miss / total) * 100, 1),
                falso_alarme=round(float(fa / total) * 100, 1), confusao=round(float(conf / total) * 100, 1),
                fala_ref_s=round(float(total * step), 1))

def acuracia_nomes(ref, hyp, step=0.01):
    """Voz conhecida: fração do tempo (de fala da verdade) em que o NOME da hipótese bate com o falante real."""
    fim = max(e for _, e, _ in ref + hyp) + 1; n = int(fim / step)
    h = np.full(n, "", dtype=object)
    for a, b, s in hyp: h[int(a / step):int(b / step)] = s
    tot = ok = 0
    for a, b, s in ref:
        seg = h[int(a / step):int(b / step)]; tot += len(seg); ok += int((seg == s).sum())
    return round(100 * ok / max(tot, 1), 1)

if __name__ == "__main__":
    if "--nomes" in sys.argv:
        print(json.dumps({"acuracia_nomes_pct": acuracia_nomes(carrega(sys.argv[1]), carrega(sys.argv[2]))})); sys.exit(0)
    a = sys.argv; col = float(a[a.index("--collar") + 1]) if "--collar" in a else 0.25
    print(json.dumps(der(carrega(a[1]), carrega(a[2]), col), ensure_ascii=False))
