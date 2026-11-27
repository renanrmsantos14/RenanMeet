---
name: verificar-diarizacao
description: Use ao mexer em crates/meet-core/src/voice (diarização, impressão de voz, vozes salvas, pré-processamento) ou nos modelos de voz — prova com DER medido em áudio sintético de verdade conhecida (online e canal único) e com o teste de voz conhecida.
---

# Verificar diarização (RenanMeet)

Prova só vale com número medido. Não use áudio de reunião real; tudo é sintético (piper pt-BR).

## Pré-requisitos (fora do repo, em `.cache/`)
- `.cache/models/{sherpa,segmentation,embedding}`: Sherpa-ONNX Linux x64 (shared), `segmentation/model.onnx` (pyannote 3.0) e `embedding/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx`.
- `.cache/voices/`: vozes piper `pt_BR-{faber,edresson,cadu,jeff}` (`.onnx` + `.onnx.json`) de `rhasspy/piper-voices` no Hugging Face.
- `.venv` com `piper-tts` e `numpy`. Disco costuma estar quase cheio: confira `df -h /home` antes.

## Passos
1. `cargo build -p meet-core --release`
2. Gerar o áudio (≈2 min por semente): `.venv/bin/python -I tools/testaudio/gen.py .cache/voices testdata/out` e, para variar, `... testdata/out <semente> sala_4,online_4` (sementes 1–3).
3. `tools/testaudio/avaliar.sh` — imprime o DER por arquivo e a média por grupo:
   - `online_*` (vozes limpas, como o canal de sistema);
   - `canal_unico_*` (4 vozes num só canal, reverberação, ruído de sala a 15 dB, uma fala sobreposta de 1,5 s) com e sem `--presencial`, número de falantes informado e automático (o reagrupamento por trecho é padrão; `--sem-refinar`, `--denoise`, `--agc` e `--dist X` servem para comparar variantes; `tools/testaudio/variantes.sh` mede uma variante nos 4 `sala_4*`).
   Registre separado: online × canal único. A variação entre execuções é grande (uma amostra só engana): use a média de ≥4 arquivos.
4. Voz conhecida (usa `RENANMEET_DATA_DIR` limpo):
   - `meetcli enroll testdata/out/cadastro.wav Renan --dono`
   - `meetcli diarize testdata/out/online_2.wav --speakers 2 --nomear "1=Ana"` (o cluster que não for Renan)
   - `meetcli diarize testdata/out/sala_4.wav --speakers 4 --saida h.json` e
     `der.py verdade.json h.json --nomes` (acurácia do nome, só Renan e Ana contam; Caio e Davi devem ficar sem nome).
5. Memória e tempo: `/usr/bin/time -v meetcli diarize ...` (pico em KB) e tempo ÷ duração; para 1 h, repetir o WAV até 3600 s e medir de verdade (a segmentação domina; o reagrupamento é O(n³) em tese, 2 s em 3,5 min).

## Critérios
- Referência atual (D4): online ≈ 1,0 %, canal único ≈ 4,9 % (nº conhecido) / 5,6 % (auto); 1 h = 9–12 min (95 % é a segmentação do Sherpa; varia com a carga da máquina) e ~540 MB de pico só no processo de análise.
- Não regredir a média do `online_*` nem a do `canal_unico_*` registradas em `docs/decisoes.md` (D4) além de 3 pontos.
- Nenhum falante desconhecido (Caio/Davi) pode receber nome de voz salva (falso positivo = 0).
- O processo de análise sai ao terminar (RAM devolvida).

## Armadilhas
- Voz TTS é mais regular que voz real: DER real tende a ser pior. Diga isso ao reportar.
- O exe do Sherpa e a lib C não devem ser linkados no mesmo binário com outro onnxruntime: mantenha o exe em subprocesso.
- `.cache/`, `testdata/out`, `.venv` são ignorados pelo git; não commite áudio nem modelos.
