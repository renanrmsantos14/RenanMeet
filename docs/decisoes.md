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
