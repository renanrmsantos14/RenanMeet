# Decisões do RenanMeet

## D1 — RenanMeet é o AtaLocal refeito (decisão do Renan)
Histórico do AtaLocal importado por merge `--allow-unrelated-histories`; o repo AtaLocal não foi tocado.
Saem whisper.cpp, llama.cpp/Qwen, React/TS/Vite e os APKs versionados. Docs antigos em `docs/legado/`.

## D2 — Tudo em Rust; interface em Slint
Protótipo mínimo igual nos três (busca, slider, lista de 500 linhas, texto multilinha, botão), release,
medido parado por 6 s (`tools/ui-prototypes/measure.sh`):

| Framework | Binário | Xvfb `:99` RSS / PSS | Tela real `:0` RSS / PSS | Renderizou |
|---|---|---|---|---|
| Slint (software, winit) | 14 MB | 16 / 11 MB | 11 / 7 MB | sim (print) |
| iced (tiny-skia) | 6 MB | 21 / 16 MB | 17 / 13 MB | sim (print) |
| egui (glow) | 6 MB | sem janela (falta GL) | 20 / 17 MB | não verificado por print |

Nota: o `:0` estava com a tela bloqueada na medição, então vale só como ordem de grandeza.
Escolha: **Slint** — menor RAM entre os que renderizaram, `ListView` virtualizado, tema claro/escuro
nativo, suporte a Android (backend android-activity) e licença royalty-free para apps de desktop/mobile.
Pontos fracos: binário maior (14 MB) e ecossistema menor; bandeja, atalho global e autostart ficam em
crates próprias (`ksni`/`tray-icon`, `global-hotkey`). Plano B: iced.
Ainda a provar: janela do Slint no Android e waveform/player (desenho custom).

## D3 — Captura
Linux: `parec` em subprocesso (sem libs de ALSA/Pulse a compilar); Windows: `cpal` (WASAPI, loopback).
Opus em Rust puro (`unsafe-libopus` + `ogg`), sem cmake. Estéreo L = mic, R = sistema; presencial = mono.
Provado no Linux (mic + monitor tocando tom sintético). Windows: só `cargo check` para MSVC.

## D4 — Diarização local e voz conhecida
**Pipeline** (roda só depois da gravação, em processo próprio, e libera tudo ao terminar): segmentação
pyannote-3.0 (Sherpa, subprocesso) → reagrupamento por trecho com embeddings CAM++ (192 dim, lib C do Sherpa
via `libloading`) → nomes por cosseno contra as vozes salvas (SQLite, limiar padrão 0,5). O Sherpa nunca é
linkado ao binário principal.

**Embedding**: CAM++ (`3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced`) venceu 5 candidatos
(CAM++ en/voxceleb, ERes2Net, TitaNet small, WeSpeaker ResNet34) no mesmo áudio.

**Reagrupamento por trecho (novo, `voice/refine.rs`)**: o clustering padrão do Sherpa errava muito no canal único.
Trocamos por trechos de até 6 s → embedding → subtrai a média da gravação → ligação média aglomerativa
(com nº de participantes, ou limiar de distância 0,75 sem ele). O limiar é sensível: 0,6 → 15,8 %, 0,75 → 7,7 %,
0,9 → 12,2 %, 1,0 → 21,5 % (medido antes do conserto de fusão de trechos abaixo; o padrão 0,75 segue o melhor) (sem nº de participantes). **Informar o nº de participantes é recomendado.**
Centralizar pela média não mudou o resultado (6,8 % nos dois); fica sempre ligado.
A revisão achou que fundir trechos encolhia spans sobrepostos (`last.end` recuava); com `max` o canal único caiu de 6,9 % para 4,9 %.
Uma voz salva só nomeia um falante por gravação (o de maior cosseno).

**Pré-processamento (RNNoise e AGC) desligado por padrão**: piorou o DER (sem 26,1 %, só denoise 35,3 %, só AGC 31,3 %,
medido ainda sem o reagrupamento). Continua disponível com `--denoise`/`--agc` no `meetcli`.

**DER** (áudio sintético: piper-tts pt_BR, 4 vozes, ~3 min; reverb + ruído rosa 15 dB e fala sobreposta no canal único):

| Cenário | Antes | Depois |
|---|---|---|
| Online (2 canais mixados), nº conhecido | 3,3 % | **1,0 %** (0,3–2,0) |
| Canal único (sala), nº conhecido | 26,1 % (10–39) | **4,9 %** (3,3–6,4) |
| Canal único, nº desconhecido | — | **5,6 %** (4,9–7,5) |

**Voz conhecida** (4 vozes cadastradas em `online_4`, reconhecidas nos outros 7 áudios com nº de falantes informado; acurácia do nome por tempo de fala):
online 94,7–95,6 %, canal único 90,6–94,7 % (média 92,8 %). Falso positivo: cadastrando só Renan e Ana, Caio e Davi ficaram sem nome
(cosseno mínimo 0,5; Ana 0,79 e Renan 0,77 nos acertos).

**Custo** (desktop de 12 núcleos, 1 h = áudio sintético repetido, sem nº de participantes): 526 s (8,8 min) com a máquina livre
e 743 s com a máquina a load 15 (outros processos rodando). A segmentação do Sherpa é ~95 % do tempo (39 s de 43 s em 208 s de
áudio); o reagrupamento leva 2 s. Pico de RAM ≈ 540 MB só durante a análise, que sai ao terminar. O app parado não carrega nada disso.
Pendente de otimização (não feito): o pico vem do carregamento do áudio inteiro + Sherpa; o denoise opcional usa ~1,5 GB em 1 h.

**Honestidade sobre a medição**: as vozes são TTS (3 masculinas parecidas) e o ruído/reverb é sintético; fala real
tende a ser mais variada entre pessoas, mas também mais irregular. Os números são da bancada, não de reunião real.
Falta ainda: DER em áudio real de domínio público e reconhecimento do Renan cadastrado com a voz dele.

## D5 — Transcrição, ata, banco e wiki (bloco 3)

- **Groq**: STT `whisper-large-v3` (`verbose_json`, segmentos) e ata com `openai/gpt-oss-120b` em `json_object` (o `llama-3.3-70b-versatile` é desligado em 16/08/2026). Retry em 429/5xx/rede respeitando `Retry-After`; 401 falha na hora sem ecoar a chave; User-Agent próprio (a Groq dá 403 ao de biblioteca). HTTP com `ureq` (rustls), multipart montado à mão: sem tokio nem reqwest.
- **Chave**: keyring do sistema (Secret Service no Linux, Credential Manager no Windows); `RENANMEET_GROQ_KEY` tem precedência (testes). Nunca no repo nem em argumento de linha de comando.
- **Blocos**: ≤ 600 s, corte no ponto mais silencioso dos últimos 30 s, Opus mono 32 kb/s (~2,4 MB, longe dos 25 MB). Alucinações do Whisper (mesma frase repetida) caem para no máximo 2 repetições.
- **Falante**: cada frase vai para o falante com mais sobreposição (mín. 0,2 s); sem sobreposição, herda o trecho de voz mais próximo (≤ 1,5 s). Transcrição (rede) e análise de voz (CPU) rodam ao mesmo tempo; a impressão de voz só é comparada às vozes salvas depois.
- **Ata**: partes de ~14 mil caracteres, ata parcial por parte e redução em JSON; prompt trata a transcrição como dado (anti-injeção). Falha na ata não perde a transcrição.
- **Banco**: um SQLite (`renanmeet.db`, WAL) para reuniões, trechos, falantes, ata, tarefas e vozes. Busca FTS5 (`unicode61 remove_diacritics 2`) com consulta sanitizada. Hora de Brasília como offset fixo −03:00 (sem horário de verão desde 2019).
- **Wiki**: a nota leva só decisões e tarefas (formato da skill `/wiki`), nunca a transcrição; falha do git não perde a nota.
- **Modelos**: baixados na primeira execução com SHA-256 fixado, `.part` e troca atômica; fora do repo. Não retoma download interrompido (reinicia).
- **Importação**: `.opus`, `.wav`, `.m4a`, `.mp3`, `.flac` viram Opus mono 16 kHz (symphonia, Rust puro) — é o caminho do Android gravando e importando no PC.
- **Não provado**: Groq real (sem chave no ambiente); só servidor local.
