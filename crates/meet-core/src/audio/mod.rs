//! Áudio: captura (Linux: PulseAudio/PipeWire; Windows: WASAPI), Ogg Opus e reamostragem.

pub mod capture;
pub mod decode;
pub mod opus;
pub mod recorder;
pub mod resample;
