//! The AI gateway: one [`LlmClient`] trait every service talks to, plus an
//! optional real Ollama backend behind the `ollama-http` feature.
//!
//! Keeping the trait tiny (a blocking chat call + an optional embedding call)
//! means [`crate::studio`], [`crate::agents`], and [`crate::ingestion`] are all
//! unit-testable against a scripted fake, and the only place that touches the
//! network is one feature-gated struct. Ollama runs on `localhost`, so **no data
//! leaves the machine** and no TLS backend is compiled in.

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("the local AI service is unavailable: {0}")]
    Transport(String),
    #[error("the AI service returned an unexpected response: {0}")]
    Protocol(String),
    #[error("this LLM client does not support {0}")]
    Unsupported(&'static str),
}

/// A minimal chat-completion + embedding abstraction.
///
/// `embed` defaults to [`GatewayError::Unsupported`] so a chat-only backend can
/// implement just `complete`; the [`crate::embedding::HashingEmbedder`] is the
/// no-model fallback for search.
pub trait LlmClient: Send + Sync {
    /// Return the assistant's raw text for `system_prompt` + `user_prompt`.
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, GatewayError>;

    /// Embed `text` into a vector, if this backend supports embeddings.
    fn embed(&self, _text: &str) -> Result<Vec<f32>, GatewayError> {
        Err(GatewayError::Unsupported("embeddings"))
    }
}

/// Default local Ollama endpoint + models (overridable by the caller).
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
pub const DEFAULT_CHAT_MODEL: &str = "llama3.2";
pub const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";

#[cfg(feature = "ollama-http")]
mod ollama {
    use super::*;
    use serde::Deserialize;

    /// A real Ollama client. Chat hits `/api/chat` with `format: "json"` (so the
    /// model is nudged toward the strict JSON our parsers expect); embeddings hit
    /// `/api/embeddings`.
    pub struct OllamaClient {
        base_url: String,
        chat_model: String,
        embed_model: String,
        http: reqwest::blocking::Client,
    }

    impl OllamaClient {
        pub fn new(base_url: impl Into<String>, chat_model: impl Into<String>) -> Self {
            Self {
                base_url: base_url.into(),
                chat_model: chat_model.into(),
                embed_model: DEFAULT_EMBED_MODEL.to_string(),
                http: reqwest::blocking::Client::new(),
            }
        }

        pub fn with_embed_model(mut self, model: impl Into<String>) -> Self {
            self.embed_model = model.into();
            self
        }
    }

    impl Default for OllamaClient {
        fn default() -> Self {
            Self::new(DEFAULT_OLLAMA_URL, DEFAULT_CHAT_MODEL)
        }
    }

    #[derive(Deserialize)]
    struct ChatResponse {
        message: ChatMessage,
    }
    #[derive(Deserialize)]
    struct ChatMessage {
        content: String,
    }
    #[derive(Deserialize)]
    struct EmbedResponse {
        embedding: Vec<f32>,
    }

    impl LlmClient for OllamaClient {
        fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, GatewayError> {
            let body = serde_json::json!({
                "model": self.chat_model,
                "stream": false,
                "format": "json",
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": user_prompt},
                ],
            });
            let resp = self
                .http
                .post(format!("{}/api/chat", self.base_url))
                .json(&body)
                .send()
                .map_err(|e| GatewayError::Transport(e.to_string()))?;
            let parsed: ChatResponse = resp
                .json()
                .map_err(|e| GatewayError::Protocol(e.to_string()))?;
            Ok(parsed.message.content)
        }

        fn embed(&self, text: &str) -> Result<Vec<f32>, GatewayError> {
            let body = serde_json::json!({ "model": self.embed_model, "prompt": text });
            let resp = self
                .http
                .post(format!("{}/api/embeddings", self.base_url))
                .json(&body)
                .send()
                .map_err(|e| GatewayError::Transport(e.to_string()))?;
            let parsed: EmbedResponse = resp
                .json()
                .map_err(|e| GatewayError::Protocol(e.to_string()))?;
            Ok(parsed.embedding)
        }
    }
}

#[cfg(feature = "ollama-http")]
pub use ollama::OllamaClient;

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use std::sync::Mutex;

    /// A scripted fake LLM: pops the next canned reply from a queue on each
    /// `complete` call. Lets every downstream service be tested without a model.
    pub struct FakeLlm {
        replies: Mutex<std::collections::VecDeque<String>>,
        /// The last (system, user) prompt pair seen, for prompt assertions.
        pub last: Mutex<Option<(String, String)>>,
    }

    impl FakeLlm {
        pub fn new(replies: impl IntoIterator<Item = &'static str>) -> Self {
            Self {
                replies: Mutex::new(replies.into_iter().map(String::from).collect()),
                last: Mutex::new(None),
            }
        }
    }

    impl LlmClient for FakeLlm {
        fn complete(&self, system: &str, user: &str) -> Result<String, GatewayError> {
            *self.last.lock().unwrap() = Some((system.to_string(), user.to_string()));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| GatewayError::Transport("fake: no scripted reply".into()))
        }
    }
}
