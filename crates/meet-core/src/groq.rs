//! Cliente mínimo da API da Groq: transcrição (Whisper) e chat (ata).
//! O User-Agent é próprio porque a Groq responde 403 ao agente padrão de bibliotecas.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const BASE: &str = "https://api.groq.com/openai/v1";
pub const STT_MODEL: &str = "whisper-large-v3";
/// `llama-3.3-70b-versatile` foi desligado em 16/08/2026; este é o que a Groq recomenda no lugar.
pub const CHAT_MODEL: &str = "openai/gpt-oss-120b";
const MAX_TRIES: u32 = 4;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SttSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub struct Groq {
    key: String,
    base: String,
    agent: ureq::Agent,
}

impl Groq {
    /// `RENANMEET_GROQ_BASE` troca o endereço da API (testes com servidor local).
    pub fn new(key: &str) -> Self {
        Self::with_base(key, &std::env::var("RENANMEET_GROQ_BASE").unwrap_or_else(|_| BASE.into()))
    }

    pub fn with_base(key: &str, base: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .user_agent(concat!("renanmeet/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .build();
        Self { key: key.trim().to_string(), base: base.trim_end_matches('/').to_string(), agent }
    }

    /// Valida a chave sem gastar cota (lista os modelos).
    pub fn check_key(&self) -> Result<()> {
        self.send(|| self.agent.get(&format!("{}/models", self.base)).set("Authorization", &self.auth()).call())
            .map(|_| ())
    }

    /// Transcreve um arquivo de áudio (Ogg Opus, WAV…). Devolve os trechos com tempo em segundos
    /// a partir do início desse arquivo.
    pub fn transcribe(&self, audio: &[u8], filename: &str, language: &str, prompt: Option<&str>) -> Result<Vec<SttSegment>> {
        let boundary = "renanmeet-boundary-7f3a9c";
        let mut body: Vec<u8> = Vec::with_capacity(audio.len() + 1024);
        let mut field = |name: &str, value: &str| {
            body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
        };
        field("model", STT_MODEL);
        field("language", language);
        field("response_format", "verbose_json");
        field("temperature", "0");
        field("timestamp_granularities[]", "segment");
        if let Some(p) = prompt {
            field("prompt", p);
        }
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(audio);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let ctype = format!("multipart/form-data; boundary={boundary}");
        let resp = self.send(|| {
            self.agent
                .post(&format!("{}/audio/transcriptions", self.base))
                .set("Authorization", &self.auth())
                .set("Content-Type", &ctype)
                .send_bytes(&body)
        })?;
        parse_segments(&resp.into_json()?)
    }

    /// Chat em modo JSON: devolve o texto da resposta (um objeto JSON).
    pub fn chat_json(&self, model: &str, system: &str, user: &str) -> Result<String> {
        let payload = json!({
            "model": model,
            "temperature": 0.2,
            "response_format": {"type": "json_object"},
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });
        let resp = self.send(|| {
            self.agent
                .post(&format!("{}/chat/completions", self.base))
                .set("Authorization", &self.auth())
                .send_json(&payload)
        })?;
        let v: Value = resp.into_json()?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("resposta do chat sem conteúdo"))
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.key)
    }

    /// Repete em 429/5xx/queda de rede (respeitando `Retry-After`); erro de cliente falha na hora.
    fn send(&self, mut call: impl FnMut() -> Result<ureq::Response, ureq::Error>) -> Result<ureq::Response> {
        let mut last = anyhow!("sem tentativa");
        for attempt in 0..MAX_TRIES {
            match call() {
                Ok(r) => return Ok(r),
                Err(ureq::Error::Status(code, r)) => {
                    let wait = r.header("retry-after").and_then(|v| v.trim().parse::<f64>().ok()).filter(|w| w.is_finite() && *w >= 0.0);
                    let detail = r.into_string().unwrap_or_default();
                    last = anyhow!("Groq respondeu {code}: {}", short_error(&detail));
                    if code == 401 {
                        bail!("chave da Groq recusada (401): confira a chave nas configurações");
                    }
                    if code != 429 && code < 500 {
                        return Err(last);
                    }
                    if attempt + 1 < MAX_TRIES {
                        std::thread::sleep(Duration::from_secs_f64(wait.unwrap_or(2f64.powi(attempt as i32 + 1)).min(60.0)));
                    }
                }
                Err(e) => {
                    last = anyhow!(e).context("falha de rede ao falar com a Groq");
                    if attempt + 1 < MAX_TRIES {
                        std::thread::sleep(Duration::from_secs(2u64.pow(attempt + 1)));
                    }
                }
            }
        }
        Err(last)
    }
}

fn short_error(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect())
}

fn parse_segments(v: &Value) -> Result<Vec<SttSegment>> {
    let segs = v["segments"].as_array().context("resposta de transcrição sem 'segments'")?;
    let mut out = Vec::with_capacity(segs.len());
    for s in segs {
        let text = s["text"].as_str().unwrap_or("").trim().to_string();
        if text.is_empty() {
            continue;
        }
        out.push(SttSegment {
            start: s["start"].as_f64().context("segmento sem 'start'")?,
            end: s["end"].as_f64().context("segmento sem 'end'")?,
            text,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_segmentos_e_ignora_vazios() {
        let v = json!({"segments": [
            {"start": 0.0, "end": 2.5, "text": " Bom dia a todos."},
            {"start": 2.5, "end": 3.0, "text": "  "},
            {"start": 3.0, "end": 5.0, "text": "Vamos começar."}]});
        let s = parse_segments(&v).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].text, "Bom dia a todos.");
    }

    #[test]
    fn erro_da_api_vira_mensagem_curta() {
        assert_eq!(short_error(r#"{"error":{"message":"model_not_found"}}"#), "model_not_found");
    }

    #[test]
    fn transcricao_manda_agente_proprio_chave_e_repete_no_429() {
        use crate::testserver::{serve, Reply};
        let ok = json!({"segments": [{"start": 1.0, "end": 2.0, "text": "Oi"}]}).to_string();
        let limite = Reply { status: 429, headers: vec![("Retry-After", "0".into())], body: br#"{"error":{"message":"rate limit"}}"#.to_vec() };
        let (base, srv) = serve(vec![limite, Reply::ok(ok)]);
        let g = Groq::with_base("  gsk_teste ", &base);
        let r = g.transcribe(b"AUDIO", "b.opus", "pt", None).unwrap();
        assert_eq!(r, vec![SttSegment { start: 1.0, end: 2.0, text: "Oi".into() }]);
        let seen = srv.join().unwrap();
        assert_eq!(seen.len(), 2, "uma repetição");
        let s = &seen[1];
        assert!(s.request_line.starts_with("POST /audio/transcriptions"));
        assert!(s.header("user-agent").unwrap().starts_with("renanmeet/"));
        assert_eq!(s.header("authorization"), Some("Bearer gsk_teste"));
        let body = String::from_utf8_lossy(&s.body);
        for t in ["name=\"model\"\r\n\r\nwhisper-large-v3", "name=\"language\"\r\n\r\npt", "verbose_json", "AUDIO"] {
            assert!(body.contains(t), "faltou {t}");
        }
    }

    #[test]
    fn chave_recusada_falha_na_hora_sem_vazar_a_chave() {
        use crate::testserver::{serve, Reply};
        let (base, srv) = serve(vec![Reply::status(401, r#"{"error":{"message":"invalid key"}}"#)]);
        let e = Groq::with_base("gsk_segredo123", &base).check_key().unwrap_err().to_string();
        assert!(e.contains("401") && !e.contains("gsk_segredo123"), "{e}");
        assert_eq!(srv.join().unwrap().len(), 1, "sem repetição");
    }

    #[test]
    fn erro_de_cliente_nao_repete() {
        use crate::testserver::{serve, Reply};
        let (base, srv) = serve(vec![Reply::status(404, r#"{"error":{"message":"model_not_found"}}"#)]);
        let e = Groq::with_base("k", &base).chat_json("x", "s", "u").unwrap_err().to_string();
        assert!(e.contains("404") && e.contains("model_not_found"), "{e}");
        assert_eq!(srv.join().unwrap().len(), 1);
    }
}
