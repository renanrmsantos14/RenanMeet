#!/usr/bin/env python3
"""Gera áudio de teste 100% sintético (piper pt-BR) com verdade conhecida.

Cenários (16 kHz mono, em testdata/out/):
  online_2  : 2 vozes, turnos limpos (como o canal de sistema numa reunião online)
  online_4  : 4 vozes, turnos limpos
  sala_4    : canal ÚNICO, 4 vozes misturadas, reverberação + ruído de sala (SNR 15 dB)
              e uma fala sobreposta de ~1,5 s (caso presencial)
  cadastro  : ~20 s lendo um texto com a voz "Renan" (faber), para o cadastro de voz
Cada cenário gera <nome>.wav e <nome>.json (verdade: [{start,end,speaker}]).
Uso: gen.py <pasta_vozes> <pasta_saida> [semente [cenario,...]]  (sem semente: conjunto base, semente 42)
"""
import json, random, sys, wave
from pathlib import Path
import numpy as np
from piper import PiperVoice

VOZES = {"Renan": "pt_BR-faber-medium", "Ana": "pt_BR-edresson-low",
         "Caio": "pt_BR-cadu-medium", "Davi": "pt_BR-jeff-medium"}
SR = 16000
FRASES = [
 "Bom dia a todos, vamos começar a reunião revisando o que ficou combinado na semana passada.",
 "Eu terminei a proposta comercial e já enviei para o cliente aprovar até sexta-feira.",
 "O orçamento do próximo trimestre precisa ser fechado antes do dia quinze.",
 "Preciso que alguém confirme a disponibilidade dos motoristas para o evento de sábado.",
 "A frota está com dois veículos na oficina e devem voltar na quarta de manhã.",
 "Podemos adiantar a entrega do relatório se a equipe financeira liberar os dados hoje.",
 "Eu prefiro manter o prazo original, porque a mudança traz risco para a qualidade.",
 "Fica decidido que a reunião de alinhamento passa a ser toda segunda às nove horas.",
 "Quem ficou responsável por falar com o fornecedor sobre o atraso das peças?",
 "Eu falo com ele ainda hoje e mando o retorno no grupo da equipe.",
 "O cliente pediu um desconto, mas acho que podemos oferecer um serviço adicional no lugar.",
 "Vamos registrar essa pendência e revisar na próxima reunião com calma.",
 "A pesquisa de satisfação mostrou melhora na pontualidade, mas ainda temos queixas de comunicação.",
 "Preciso de mais uma semana para concluir os testes antes de publicar a atualização.",
 "Ótimo, então fechamos assim e cada um segue com a sua parte até o fim do mês.",
 "Alguém tem mais algum ponto que queira discutir antes de encerrarmos por hoje?",
]
_cache = {}
def voz(n):
    if n not in _cache:
        _cache[n] = PiperVoice.load(str(VOZ_DIR / f"{VOZES[n]}.onnx"))
    return _cache[n]

def resample(x, a, b):
    n = int(round(len(x) * b / a)); X = np.fft.rfft(x)
    Y = np.zeros(n // 2 + 1, dtype=complex); m = min(len(X), len(Y)); Y[:m] = X[:m]
    return np.fft.irfft(Y, n) * (n / len(x))

def sintetiza(n, texto):
    v = voz(n); partes = [np.frombuffer(c.audio_int16_bytes, dtype=np.int16) for c in v.synthesize(texto)]
    x = np.concatenate(partes).astype(np.float64) / 32768.0
    x = resample(x, v.config.sample_rate, SR)
    return x / (np.abs(x).max() + 1e-9) * 0.7

def turnos(falantes, n, rng):
    ordem, ult = [], None
    for _ in range(n):
        f = rng.choice([s for s in falantes if s != ult]); ult = f
        ordem.append((f, " ".join(rng.sample(FRASES, 2))))
    return ordem

def monta(falantes, n, rng, sobreposicao=None):
    """Retorna (áudio, verdade). sobreposicao = índice do turno que começa 1,5 s antes do anterior acabar."""
    sinal = np.zeros(SR * 600); verdade = []; t = 0.5
    for i, (f, texto) in enumerate(turnos(falantes, n, rng)):
        x = sintetiza(f, texto) * rng.uniform(0.7, 1.0)
        ini = t - (1.5 if i == sobreposicao else 0.0)
        a = int(ini * SR); sinal[a:a + len(x)] += x
        verdade.append({"start": round(ini, 3), "end": round(ini + len(x) / SR, 3), "speaker": f})
        t = ini + len(x) / SR + rng.uniform(0.4, 1.0)
    return sinal[: int((t + 0.5) * SR)], verdade

def sala(x, rng):
    nrng = np.random.default_rng(7)
    n = int(0.35 * SR); ir = nrng.standard_normal(n) * np.exp(-np.arange(n) / (0.07 * SR)); ir[0] = 1.0
    y = np.convolve(x, ir / np.abs(ir).sum() * 2.5)[: len(x)]
    ruido = np.fft.irfft(np.fft.rfft(nrng.standard_normal(len(x))) / np.sqrt(np.maximum(np.arange(len(x) // 2 + 1), 1)), len(x))
    pot = lambda s: float(np.mean(s ** 2))
    return y + ruido * np.sqrt(pot(y) / pot(ruido) / 10 ** (15 / 10))

def grava(p, x):
    x = x / max(np.abs(x).max(), 1e-9) * 0.9
    with wave.open(str(p), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes((x * 32767).astype(np.int16).tobytes())

if __name__ == "__main__":
    VOZ_DIR, OUT = Path(sys.argv[1]), Path(sys.argv[2]); OUT.mkdir(parents=True, exist_ok=True)
    semente = int(sys.argv[3]) if len(sys.argv) > 3 else 42
    so = sys.argv[4].split(',') if len(sys.argv) > 4 else None
    suf = f'_s{semente}' if len(sys.argv) > 3 else ''
    rng = random.Random(semente)
    for nome, falantes, n, ov, amb in [("online_2", ["Renan", "Ana"], 16, None, False),
                                       ("online_4", ["Renan", "Ana", "Caio", "Davi"], 20, None, False),
                                       ("sala_4", ["Renan", "Ana", "Caio", "Davi"], 20, 9, True)]:
        if so and nome not in so: continue
        x, v = monta(falantes, n, rng, ov)
        if amb: x = sala(x, rng)
        grava(OUT / f"{nome}{suf}.wav", x); (OUT / f"{nome}{suf}.json").write_text(json.dumps(v, ensure_ascii=False, indent=1))
        print(nome, f"{len(x)/SR:.0f} s", len(v), "turnos")
    if so: sys.exit(0)
    cad = np.concatenate([sintetiza("Renan", t) for t in [
        "Meu nome é Renan e estou cadastrando a minha voz neste aplicativo de atas de reunião.",
        "Hoje o dia está bonito, o trânsito estava tranquilo e a equipe chegou cedo ao escritório.",
        "Vou ler este texto com calma para que o sistema aprenda a reconhecer a minha forma de falar."]])
    grava(OUT / "cadastro.wav", cad); print("cadastro", f"{len(cad)/SR:.0f} s")
