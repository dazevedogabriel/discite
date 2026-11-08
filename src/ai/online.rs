//! IA online por API no formato da OpenAI (`/chat/completions`), que Gemini,
//! Groq, OpenRouter, Cerebras, Mistral, OpenAI e também Ollama e LM Studio
//! aceitam. Usa as mesmas instruções e notas da IA local, e a resposta passa
//! pelo mesmo leitor (`prompt::Stream`), que confere cada cartão com as notas:
//! a regra de só usar o que está escrito vale igual.

use super::engine::Event;
use super::prompt::{self, Chat, Out};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Configuração salva da IA online.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Api {
    /// Id do serviço da lista (`PROVIDERS`) ou "custom".
    pub provider: String,
    pub url: String,
    pub key: String,
    pub model: String,
}

impl Api {
    pub fn ready(&self) -> bool {
        !self.url.trim().is_empty() && !self.model.trim().is_empty()
    }
}

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    /// Onde pegar a chave.
    pub key_url: &'static str,
    /// Modelo sugerido (o usuário pode trocar; "Buscar modelos" lista os do serviço).
    pub model: &'static str,
    /// Tem plano grátis (com limite de pedidos).
    pub free: bool,
}

/// shortcut: os modelos sugeridos e os planos grátis mudam com o tempo; o
/// botão "Buscar modelos" pega a lista atual do serviço.
pub const PROVIDERS: [Provider; 7] = [
    Provider { id: "gemini", name: "Google Gemini", url: "https://generativelanguage.googleapis.com/v1beta/openai", key_url: "https://aistudio.google.com/apikey", model: "gemini-2.5-flash", free: true },
    Provider { id: "groq", name: "Groq", url: "https://api.groq.com/openai/v1", key_url: "https://console.groq.com/keys", model: "llama-3.3-70b-versatile", free: true },
    Provider { id: "openrouter", name: "OpenRouter", url: "https://openrouter.ai/api/v1", key_url: "https://openrouter.ai/keys", model: "deepseek/deepseek-chat-v3-0324:free", free: true },
    Provider { id: "cerebras", name: "Cerebras", url: "https://api.cerebras.ai/v1", key_url: "https://cloud.cerebras.ai", model: "llama-3.3-70b", free: true },
    Provider { id: "mistral", name: "Mistral", url: "https://api.mistral.ai/v1", key_url: "https://console.mistral.ai/api-keys", model: "mistral-small-latest", free: true },
    Provider { id: "openai", name: "OpenAI", url: "https://api.openai.com/v1", key_url: "https://platform.openai.com/api-keys", model: "gpt-4.1-mini", free: false },
    Provider { id: "custom", name: "", url: "http://localhost:11434/v1", key_url: "", model: "", free: false },
];

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

fn agent(read_timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(read_timeout)))
        .http_status_as_error(false)
        .user_agent(concat!("Discite/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn endpoint(api: &Api, path: &str) -> String {
    format!("{}/{path}", api.url.trim().trim_end_matches('/'))
}

/// Mensagem de erro do serviço (o corpo costuma ter `{"error": {"message": …}}`).
fn http_error(status: u16, body: &str) -> anyhow::Error {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").or_else(|| v.pointer("/0/error/message")).and_then(|m| m.as_str()).map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(300).collect());
    anyhow!("HTTP {status}: {}", msg.trim())
}

/// Os modelos que o serviço oferece (`GET /models`).
pub fn list_models(api: &Api) -> Result<Vec<String>> {
    let mut req = agent(30).get(&endpoint(api, "models"));
    if !api.key.trim().is_empty() {
        req = req.header("Authorization", &format!("Bearer {}", api.key.trim()));
    }
    let resp = req.call()?;
    let status = resp.status().as_u16();
    let body = resp.into_body().read_to_string()?;
    if status != 200 {
        return Err(http_error(status, &body));
    }
    let v: serde_json::Value = serde_json::from_str(&body)?;
    let mut ids: Vec<String> = v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str()).map(|s| s.trim_start_matches("models/").to_owned()).collect()).unwrap_or_default();
    // Os grátis da OpenRouter (":free") primeiro; o resto em ordem alfabética.
    ids.sort_by(|a, b| (!a.ends_with(":free"), a).cmp(&(!b.ends_with(":free"), b)));
    Ok(ids)
}

/// Gera com a API, mandando cada cartão ou questão assim que o objeto fecha.
#[allow(clippy::too_many_arguments)]
pub fn run(api: &Api, chat: &Chat, shape: &str, ids: Vec<i64>, notes: &str, questions: bool, count: u32, cancel: &AtomicBool, send: &dyn Fn(Event)) -> Result<f32> {
    send(Event::Loading);
    let start = Instant::now();
    // Sem a gramática da IA local, o formato vai por escrito no pedido.
    let user = format!("{}\n\n{shape}", chat.user);
    let mut body = serde_json::json!({
        "model": api.model.trim(),
        "messages": [{"role": "system", "content": chat.system}, {"role": "user", "content": user}],
        "stream": true,
        "temperature": 0.5,
        // Sem "max_tokens": nos modelos que pensam antes (Gemini 2.5, os
        // "reasoning"), o raciocínio gasta o mesmo limite e a resposta saía vazia.
        "response_format": {"type": "json_object"},
    });
    let call = |body: &serde_json::Value| {
        let mut req = agent(120).post(&endpoint(api, "chat/completions")).header("Content-Type", "application/json");
        if !api.key.trim().is_empty() {
            req = req.header("Authorization", &format!("Bearer {}", api.key.trim()));
        }
        req.send(body.to_string())
    };
    let mut resp = call(&body)?;
    // Alguns modelos não aceitam o modo JSON: tenta de novo sem ele.
    if resp.status().as_u16() == 400 {
        body.as_object_mut().unwrap().remove("response_format");
        resp = call(&body)?;
    }
    let status = resp.status().as_u16();
    if status != 200 {
        let text = resp.into_body().read_to_string().unwrap_or_default();
        return Err(http_error(status, &text));
    }
    send(Event::Writing);
    let mut stream = prompt::Stream::new(ids, notes, questions);
    let mut made = 0u32;
    // Para explicar uma resposta sem cartões: o começo do texto e por que parou.
    let (mut text, mut finish) = (String::new(), String::new());
    let reader = std::io::BufReader::new(resp.into_body().into_reader());
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let line = line?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
        if let Some(e) = v.pointer("/error/message").and_then(|m| m.as_str()) {
            bail!("{e}");
        }
        if let Some(f) = v.pointer("/choices/0/finish_reason").and_then(|f| f.as_str()) {
            finish = f.to_owned();
        }
        let Some(piece) = v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()).filter(|c| !c.is_empty()) else {
            // Pensando (raciocínio) ou só metadados: mostra que segue trabalhando.
            send(Event::Writing);
            continue;
        };
        if text.len() < 2000 {
            text.push_str(piece);
        }
        for out in stream.feed(piece) {
            // O teto que a gramática garante na IA local.
            if made >= count {
                break;
            }
            made += 1;
            send(match out {
                Out::Card(c) => Event::Card(c),
                Out::Question(q) => Event::Question(q),
            });
        }
        send(Event::Writing);
        if made >= count {
            break;
        }
    }
    if made == 0 {
        crate::diag::warn(format!("AI online: no cards; finish={finish:?}; answer starts: {}", text.chars().take(600).collect::<String>()));
        if finish == "length" {
            bail!("the model ran out of room before writing the cards (it may have spent it thinking); ask for fewer cards or pick another model");
        }
        if text.trim().is_empty() {
            bail!("the model answered with no text (finish: {finish})");
        }
    }
    Ok(start.elapsed().as_secs_f32())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_comes_from_the_body() {
        let e = http_error(429, r#"{"error": {"message": "Rate limit reached"}}"#);
        assert_eq!(e.to_string(), "HTTP 429: Rate limit reached");
        let e = http_error(500, "boom");
        assert_eq!(e.to_string(), "HTTP 500: boom");
    }

    #[test]
    fn endpoint_ignores_trailing_slash() {
        let api = Api { url: "https://api.groq.com/openai/v1/".into(), ..Default::default() };
        assert_eq!(endpoint(&api, "models"), "https://api.groq.com/openai/v1/models");
    }
}
