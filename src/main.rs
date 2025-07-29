//! Entry point for the Jarvis Rust implementation.
//!
//! This binary replicates the core functionality of the Python prototype:
//!
//!  * Listens for a wake word (default "Jarvis") using offline speech
//!    recognition via the Vosk library.
//!  * Once awakened, enters a conversation loop where it listens for
//!    commands, consults a local language model via Ollama and speaks
//!    the response out loud.
//!  * Supports two tools—`shell_task` and `codex_cli_task`—which the
//!    language model can invoke by returning a JSON object. When the
//!    user says "shadow" the assistant goes back to sleep.
//!
//! The program is highly configurable via environment variables:
//!
//!  * `VOSK_MODEL_PATH` (**required**): path to a downloaded Vosk model.
//!  * `MODEL_NAME` (optional): name of the local LLM served by Ollama.
//!  * `VOICE_NAME` (optional): partial match for selecting a specific TTS voice.
//!  * `TRIGGER_WORD` (optional): word or phrase used to wake Jarvis.
//!  * `CONVERSATION_TIMEOUT` (optional): seconds of inactivity before
//!    returning to idle.
//!  * `MIC_INDEX`/`MIC_NAME_KEYWORD` (optional): control which input
//!    device the recogniser uses (see `speech.rs` for details).

use std::env;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

mod agent;
mod speech;
mod tools;
mod tts_engine;

use agent::Agent;
use speech::SpeechRecognizer;
use tts_engine::TtsEngine;

// Note: we used to filter out common filler words ("the", "uh", "um", etc.)
// from the beginning and end of recognised phrases to reduce false
// activations. However, some users found this overly aggressive and
// confusing when legitimate words were removed. The current
// implementation processes the transcript as‑is without trimming
// specific tokens.

/// Single tokens that are likely to be false positives from the speech
/// recogniser. When the entire recognised text matches one of these
/// strings exactly (case insensitive), Jarvis will treat the result as
/// silence and continue listening. This helps filter out spurious
/// words like "the" that Vosk sometimes produces when the microphone
/// is quiet. These tokens are ignored only when they constitute the
/// entire transcript; they are not removed from legitimate commands.
const NOISE_WORDS: &[&str] = &["the", "uh", "um", "a"];

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from `.env` if present.
    dotenvy::dotenv().ok();
    env_logger::init();

    // Retrieve required and optional configuration.
    let model_path = env::var("VOSK_MODEL_PATH")
        .context("VOSK_MODEL_PATH environment variable must point to a Vosk model directory")?;
    let model_name = env::var("MODEL_NAME").unwrap_or_else(|_| "qwen3:1.7b".to_string());
    let trigger_word = env::var("TRIGGER_WORD").unwrap_or_else(|_| "jarvis".to_string());
    let timeout_secs = env::var("CONVERSATION_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);
    let voice_name = env::var("VOICE_NAME").ok();

    // Initialise audio input and speech recognition.
    let recogniser = SpeechRecognizer::new(&model_path)?;

    // Initialise TTS. If a voice is specified attempt to select it.
    let mut tts = TtsEngine::new()?;
    if let Some(name) = voice_name {
        match tts.set_voice_by_name(&name) {
            Ok(_) => log::info!("Using voice: {}", name),
            Err(e) => log::warn!("Failed to set voice '{}': {e}. Falling back to default.", name),
        }
    }

    // Initialise the language model client and agent.
    let agent = Agent::new(&model_name).await?;

    // Fixed audio capture durations. In idle mode we listen for 5 seconds
    // to detect the wake word. In conversation mode we record up to 10
    // seconds for each user utterance. These durations were determined
    // empirically to balance latency and completeness. If you need
    // finer control over these values you can modify them here.
    let idle_listen_secs: u64 = 5;
    let convo_listen_secs: u64 = 10;

    // Conversation state.
    let mut conversation_mode = false;
    let mut last_interaction = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    log::info!("Jarvis initialised. Waiting for wake word '{}'.", trigger_word);

    loop {
        if !conversation_mode {
            // In idle mode we periodically listen for a short phrase and
            // check if it contains the trigger word. Using a short
            // duration reduces latency while keeping CPU usage low.
            // Listen for up to `idle_listen_secs` seconds of audio while idle. This captures
            // most wake‑word utterances without clipping.
            match recogniser.listen_for_phrase(Duration::from_secs(idle_listen_secs)) {
                Ok(transcript) => {
                    log::debug!("Idle recognised transcript: {}", transcript);
                    let trimmed = transcript.trim();
                    if !trimmed.is_empty() {
                        let lower = trimmed.to_lowercase();
                        // If the entire transcript is a known noise word, ignore it.
                        if NOISE_WORDS.contains(&lower.as_str()) {
                            // Treat as silence
                        } else {
                            // Check whether the wake word appears anywhere in the transcript.
                            if lower.contains(&trigger_word.to_lowercase()) {
                                log::info!("Wake word detected: {}", trimmed);
                                // Acknowledge the user and switch to conversation mode.
                                tts.speak("Yes sir?").await.ok();
                                conversation_mode = true;
                                last_interaction = Instant::now();
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Speech recognition error in idle mode: {e}");
                }
            }
            continue;
        } else {
            // Conversation mode: listen for a command. If no speech is
            // recognised within the timeout window we drop back to idle.
            // In conversation mode record up to `convo_listen_secs` seconds of audio to ensure
            // full commands are captured. Adjust this value to balance responsiveness and completeness.
            match recogniser.listen_for_phrase(Duration::from_secs(convo_listen_secs)) {
                Ok(command) => {
                    log::debug!("Raw recognised transcript: {}", command);
                    let trimmed = command.trim();
                    if trimmed.is_empty() {
                        // No speech captured this round. If we've been idle longer than the
                        // configured timeout then exit conversation mode.
                        if last_interaction.elapsed() > timeout {
                            log::info!("Conversation timeout. Returning to idle mode.");
                            conversation_mode = false;
                        }
                    } else {
                        last_interaction = Instant::now();
                        // Convert to lowercase for shadow detection and noise filtering.
                        let lower = trimmed.to_lowercase();
                        // If the entire transcript is a noise word, ignore it and continue listening.
                        if NOISE_WORDS.contains(&lower.as_str()) {
                            continue;
                        }
                        // Check for the special "shadow" phrase which tells Jarvis to go back to
                        // sleep immediately.
                        if lower.contains("shadow") {
                            tts.speak("Going silent.").await.ok();
                            conversation_mode = false;
                            continue;
                        }
                        log::info!("User command: {}", trimmed);
                        // Delegate to the language model for all commands. We no longer filter
                        // based on specific keywords; instead we rely on the language model's
                        // built‑in reasoning and our existing timeout mechanism to avoid
                        // pathological hangs. The `Agent` implementation ensures that
                        // "think" blocks and Markdown are stripped before speaking, and
                        // imposes a timeout on long running requests.
                        let mut reply = agent
                            .handle_command(trimmed)
                            .await
                            .context("failed to handle command via agent")?;
                        // Provide a fallback if the model returns an empty string.
                        if reply.trim().is_empty() {
                            reply = "I'm sorry, I didn't understand. Please try again.".to_string();
                        }
                        log::info!("Assistant response: {}", reply);
                        tts.speak(&reply).await.ok();
                    }
                }
                Err(e) => {
                    log::warn!("Speech recognition error in conversation mode: {e}");
                    // If recognition fails repeatedly we still respect the
                    // timeout to avoid getting stuck.
                    if last_interaction.elapsed() > timeout {
                        conversation_mode = false;
                    }
                }
            }
        }
    }
}