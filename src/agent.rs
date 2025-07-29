//! High level language model interface for Jarvis.
//!
//! This module wraps the [`ollama-rs`](https://crates.io/crates/ollama-rs)
//! client and encodes a simple prompting strategy to call external
//! functions when requested by the model. The Python version used
//! LangChain's tool‑calling agent; here we manually instruct the LLM to
//! return either plain text or a JSON object identifying a tool to run.

use anyhow::{Context, Result};
use ollama_rs::{generation::completion::request::GenerationRequest, Ollama};
use serde_json::Value;

use crate::tools;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Minimal agent that communicates with a local LLM via Ollama.
pub struct Agent {
    client: Ollama,
    model: String,
}

impl Agent {
    /// Construct a new agent for the given model name. The Ollama
    /// client will connect to the default endpoint at
    /// `http://localhost:11434`. To change the endpoint you can set
    /// the `OLLAMA_HOST` and `OLLAMA_PORT` environment variables
    /// recognised by the underlying crate.
    pub async fn new(model: &str) -> Result<Self> {
        let client = Ollama::default();
        Ok(Self {
            client,
            model: model.to_string(),
        })
    }

    /// Send the user's spoken command to the language model and return a
    /// textual response. The model is instructed to either answer
    /// directly or emit a JSON object describing a tool call. When a
    /// tool call is requested we execute the appropriate function and
    /// return its output to the user.
    pub async fn handle_command(&self, user_input: &str) -> Result<String> {
        // System prompt describing tool usage. This keeps the prompt
        // concise while conveying the essential semantics of each
        // available tool. The assistant is told not to include any
        // additional commentary when returning JSON.
        const SYSTEM_PROMPT: &str = "You are Jarvis, a helpful AI assistant.\n\
Use `shell_task` for raw shell commands like 'ls', 'pwd', 'cat', 'date' or 'find'.\n\
Use `codex_cli_task` only for writing or scaffolding code via the Codex CLI, not for running system commands.\n\
When you need to call a tool, respond with **only** a JSON object of the form:\n\
{\"tool\": \"tool_name\", \"arguments\": {\"command\": \"...\"}}\n\
Do not include any other text, tags or explanations around the JSON (no `<think>` tags).\n\
If no tool is required, answer briefly in plain sentences. Do not use Markdown formatting,\ncode blocks, backticks or other special markup in your answers; just write the sentence(s).";

        // Compose the combined prompt. We embed the system prompt
        // directly into the user prompt rather than using the
        // `system_prompt` method on `GenerationRequest` so that older
        // versions of ollama‑rs will behave consistently.
        let prompt = format!("{}\n\nUser: {}\nAssistant:", SYSTEM_PROMPT, user_input);
        log::debug!("LLM prompt: {}", prompt);

        let request = GenerationRequest::new(self.model.clone(), prompt);
        use tokio::time::{timeout, Duration};
        // Limit the time spent waiting for the language model. If the
        // request exceeds this timeout we return a fallback response.
        let response = match timeout(Duration::from_secs(15), self.client.generate(request)).await {
            Ok(res) => res.context("failed to query local language model")?,
            Err(_) => {
                return Ok("The request to the language model timed out. Please try again.".to_string());
            }
        };
        log::debug!("Raw LLM response: {}", response.response);

        // Trim whitespace. The model might emit trailing newlines.
        let mut answer = response.response.trim().to_string();
        log::debug!("Trimmed answer: {}", answer);

        // Check for a <think>...</think> block. If present, capture it
        // separately and remove it from the answer. The thinking text
        // will be stored in ~/.jarvis/jarvis.think for later
        // inspection. We do not expose this to the end user but it
        // can be accessed via logs or by reading the file.
        if let Some(start) = answer.find("<think>") {
            if let Some(end) = answer.find("</think>") {
                let think_start = start + "<think>".len();
                let think_end = end;
                let think_text = answer[think_start..think_end].trim();
                // Write the think text to ~/.jarvis/jarvis.think
                if let Ok(home) = env::var("HOME") {
                    let jarvis_dir = PathBuf::from(&home).join(".jarvis");
                    // Try to create the directory; ignore errors
                    let _ = fs::create_dir_all(&jarvis_dir);
                    let think_file = jarvis_dir.join("jarvis.think");
                    let _ = fs::write(&think_file, think_text);
                }
                log::debug!("Captured think block: {}", think_text);
                // Remove the think block from the answer by taking
                // everything after </think>
                let remainder = &answer[end + "</think>".len()..];
                answer = remainder.trim_start().to_string();
                log::debug!("Answer after removing think block: {}", answer);
            }
        }

        // Strip any markdown fences or backticks from the answer. The
        // model sometimes wraps its plain responses in triple
        // backticks or uses inline code formatting. We remove both
        // fenced code blocks and inline backticks to ensure the
        // spoken response is clean.
        if answer.contains("```") {
            let mut cleaned = String::new();
            let mut in_code = false;
            for line in answer.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("```") {
                    in_code = !in_code;
                    continue;
                }
                if !in_code {
                    cleaned.push_str(line);
                    cleaned.push('\n');
                }
            }
            answer = cleaned.trim().to_string();
            log::debug!("Answer after removing code fences: {}", answer);
        }
        // Remove any remaining single backtick characters used for
        // inline code.
        if answer.contains('`') {
            answer = answer.replace('`', "");
            log::debug!("Answer after removing inline backticks: {}", answer);
        }
        // The model sometimes prefixes the JSON tool call with explanatory
        // markup or `<think>` blocks. Attempt to extract the tool call
        // JSON by searching for the key "tool" and then balancing
        // braces to obtain a complete JSON object. This is more
        // reliable than taking the first and last braces since the
        // assistant's reasoning may itself contain nested braces.
        if let Some(start) = answer.find("\"tool\"") {
            // Find the opening brace preceding the "tool" key.
            let mut brace_start = None;
            for (i, ch) in answer[..start].char_indices().rev() {
                if ch == '{' {
                    brace_start = Some(i);
                    break;
                }
            }
            if let Some(start_idx) = brace_start {
                // Starting from start_idx, scan forward counting braces
                let mut brace_count = 0;
                let mut end_idx = None;
                for (i, ch) in answer[start_idx..].char_indices() {
                    match ch {
                        '{' => brace_count += 1,
                        '}' => {
                            brace_count -= 1;
                            if brace_count == 0 {
                                end_idx = Some(start_idx + i);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(end_idx) = end_idx {
                    let json_slice = &answer[start_idx..=end_idx];
                    log::debug!("Found JSON slice: {}", json_slice);
                    if let Ok(json) = serde_json::from_str::<Value>(json_slice) {
                        if let Some(tool_name) = json.get("tool").and_then(|v| v.as_str()) {
                            log::debug!("Parsed tool call: {}", tool_name);
                            match tool_name {
                                "shell_task" => {
                                    log::debug!("Executing shell_task with args: {:?}", json.get("arguments"));
                                    if let Some(args) = json.get("arguments") {
                                        if let Some(command) = args.get("command").and_then(|v| v.as_str()) {
                                            let result = tools::run_shell_task(command)?;
                                            log::debug!("shell_task result: {}", result);
                                            return Ok(result);
                                        }
                                    }
                                }
                                "codex_cli_task" => {
                                    log::debug!("Executing codex_cli_task with args: {:?}", json.get("arguments"));
                                    if let Some(args) = json.get("arguments") {
                                        if let Some(command) = args.get("command").and_then(|v| v.as_str()) {
                                            // Intercept simple shell commands that should be run via shell_task instead
                                            let cmd_lower = command.trim().to_lowercase();
                                            let simple_shells = ["date", "ls", "pwd", "cat", "find", "uptime"];
                                            if simple_shells.iter().any(|c| cmd_lower == *c || cmd_lower.starts_with(&format!("{} ", c))) {
                                                log::debug!("Redirecting codex_cli_task '{}' to shell_task", command);
                                                let result = tools::run_shell_task(command)?;
                                                log::debug!("shell_task result: {}", result);
                                                return Ok(result);
                                            }
                                            let result = tools::run_codex_cli(command)?;
                                            log::debug!("codex_cli_task result: {}", result);
                                            return Ok(result);
                                        }
                                    }
                                }
                                _ => {
                                    // Unknown tool; fall through to return raw answer
                                }
                            }
                        }
                    }
                }
            }
        }
        // At this point no tool call was detected, so we will return
        // the cleaned answer. However, if the answer is excessively
        // long (indicating the model is uncertain or verbose) we
        // substitute a generic clarification request instead. This
        // prevents long monologues from blocking the UI.
        {
            let max_chars = 300;
            let max_words = 50;
            let word_count = answer.split_whitespace().count();
            if answer.len() > max_chars || word_count > max_words {
                return Ok("I'm sorry, I didn't quite understand. Please try again with a simpler command.".to_string());
            }
        }
        // If the answer is completely empty after stripping, return a
        // default clarification message instead of an empty string. An
        // empty answer can cause the TTS backend to hang.
        if answer.trim().is_empty() {
            return Ok("I didn't catch that. Could you repeat your command?".to_string());
        }
        Ok(answer)
    }
}