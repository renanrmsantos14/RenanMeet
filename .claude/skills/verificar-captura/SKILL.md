---
name: verificar-captura
description: "Verifica a captura de áudio (mic + áudio do sistema, Opus, reamostragem, decode). Use após mudar crates/meet-core/src/audio/** ou quando pedirem para testar a gravação."
---
# Verificar captura (RenanMeet)

Nunca grave reunião real: o áudio do sistema é um tom sintético; o mic fica em silêncio.

1. `cargo build -p meet-core --release && cargo test -p meet-core --release` (opus, resample).
2. Gerar um tom: `ffmpeg -f lavfi -i sine=frequency=440:duration=6 tom.wav` (ou use qualquer WAV curto de teste).
3. Tocar e gravar ao mesmo tempo (Linux PipeWire/Pulse):
   `(paplay tom.wav &); sleep 0.3; ./target/release/meetcli record online 3 v.opus`
   O JSON final deve ter `gaps_mic: 0` e `gaps_system: 0`.
4. Separar os canais e medir:
   `meetcli wav v.opus v_dir.wav dir` (sistema) e `meetcli wav v.opus v_esq.wav esq` (mic).
   Esperado: RMS do canal `dir` ≫ 1000 (medido 5524) e do `esq` ≈ 0 (medido 12, mic parado).
5. Presencial: `meetcli record presencial 3 p.opus` gera mono.
6. Windows: só `cargo check --target x86_64-pc-windows-msvc -p meet-core` (sem teste real; diga isso no PR).

## Armadilhas
- Sem `parec` no PATH a gravação falha: confira `pactl info`.
- Disco cheio (`df -h /home`) corrompe o `.opus`.
