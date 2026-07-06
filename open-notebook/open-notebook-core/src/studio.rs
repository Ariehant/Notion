//! Studio: AI content transformations over your notes — summarize, answer a
//! question from context, or rewrite text to an instruction.
//!
//! These are free-text (not JSON) LLM calls, so the value here is in the
//! prompts: each builder is pure and unit-tested, and the response is only
//! trimmed. Retrieval-augmented answering is composed at the call site — run a
//! [`crate::memory`] search, concatenate the hits, and pass them as `context`.

use crate::gateway::{GatewayError, LlmClient};

/// Guardrail so a runaway note cannot blow the model's context window. Callers
/// doing RAG should pass already-retrieved, relevant chunks rather than a whole
/// vault; this is a backstop, not the primary bound.
const MAX_CONTEXT_CHARS: usize = 24_000;

pub struct StudioService;

impl Default for StudioService {
    fn default() -> Self {
        Self
    }
}

impl StudioService {
    pub fn new() -> Self {
        Self
    }

    /// Summarize `text` into a few sentences.
    pub fn summarize<L: LlmClient>(&self, llm: &L, text: &str) -> Result<String, GatewayError> {
        let system = "You are a concise assistant. Summarize the user's text in 2-4 sentences. \
             Output only the summary, with no preamble.";
        let out = llm.complete(system, &clamp(text))?;
        Ok(out.trim().to_string())
    }

    /// Answer `question` using only the supplied `context` (RAG). The prompt
    /// tells the model to say so when the context is insufficient, to curb
    /// hallucination.
    pub fn answer<L: LlmClient>(
        &self,
        llm: &L,
        context: &str,
        question: &str,
    ) -> Result<String, GatewayError> {
        let system = "You answer strictly from the provided context. If the context does not \
             contain the answer, say you don't have enough information. Do not invent facts.";
        let user = format!(
            "Context:\n{}\n\nQuestion: {}",
            clamp(context),
            question.trim()
        );
        Ok(llm.complete(system, &user)?.trim().to_string())
    }

    /// Rewrite `text` according to `instruction` (e.g. "make this a bulleted
    /// list", "tighten to one paragraph").
    pub fn transform<L: LlmClient>(
        &self,
        llm: &L,
        text: &str,
        instruction: &str,
    ) -> Result<String, GatewayError> {
        let system = "You transform the user's text per their instruction. Output only the \
             transformed text, preserving meaning and any important detail.";
        let user = format!(
            "Instruction: {}\n\nText:\n{}",
            instruction.trim(),
            clamp(text)
        );
        Ok(llm.complete(system, &user)?.trim().to_string())
    }
}

/// Truncate on a char boundary so a multi-byte glyph is never split.
fn clamp(text: &str) -> String {
    if text.len() <= MAX_CONTEXT_CHARS {
        return text.to_string();
    }
    let mut end = MAX_CONTEXT_CHARS;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::testutil::FakeLlm;

    #[test]
    fn summarize_trims_and_returns_model_output() {
        let llm = FakeLlm::new(["  A tidy summary.  "]);
        assert_eq!(
            StudioService.summarize(&llm, "long text").unwrap(),
            "A tidy summary."
        );
    }

    #[test]
    fn answer_embeds_context_and_question_in_prompt() {
        let llm = FakeLlm::new(["The deadline is Friday."]);
        let out = StudioService
            .answer(&llm, "The project is due Friday.", "When is it due?")
            .unwrap();
        assert_eq!(out, "The deadline is Friday.");
        let (system, user) = llm.last.lock().unwrap().clone().unwrap();
        assert!(system.contains("strictly from the provided context"));
        assert!(user.contains("The project is due Friday."));
        assert!(user.contains("When is it due?"));
    }

    #[test]
    fn transform_puts_instruction_before_text() {
        let llm = FakeLlm::new(["- one\n- two"]);
        StudioService
            .transform(&llm, "one and two", "make a bulleted list")
            .unwrap();
        let (_, user) = llm.last.lock().unwrap().clone().unwrap();
        let instr = user.find("make a bulleted list").unwrap();
        let text = user.find("one and two").unwrap();
        assert!(instr < text, "instruction should precede the text");
    }

    #[test]
    fn clamp_caps_long_input_without_splitting_chars() {
        let big = "é".repeat(MAX_CONTEXT_CHARS); // 2 bytes each ⇒ well over the cap
        let clamped = clamp(&big);
        assert!(clamped.len() <= MAX_CONTEXT_CHARS + 4); // + ellipsis bytes
        assert!(clamped.ends_with('…'));
    }
}
