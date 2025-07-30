//! Text‑to‑speech abstraction built on top of the [`tts`] crate.
//!
//! The Python implementation of Jarvis relied on RHVoice for TTS. In Rust we
//! take advantage of the [`tts`] crate, which delegates synthesis to the
//! underlying operating system (Speech Dispatcher on Linux, SAPI on
//! Windows, AVFoundation on macOS). This module exposes a simple
//! [`TtsEngine`] type that can speak arbitrary strings and optionally
//! select a voice by name.

use anyhow::{anyhow, Context, Result};
use tts::Tts;

/// Wrapper around the [`tts`] crate providing convenience methods for
/// speaking text and selecting a specific voice.
use tokio::process::{Child, Command};

pub struct TtsEngine {
    tts: Tts,
    /// Whether to use the external RHVoice CLI instead of the built‑in
    /// `tts` crate. This is configured via the VOICE_ENGINE
    /// environment variable. If set to "rhvoice" (case insensitive),
    /// Jarvis will spawn the `rhvoice-test` command for speech
    /// synthesis. This can reduce latency on some systems and
    /// matches the behaviour of the original Python prototype.
    use_rhvoice: bool,
    /// Handle to the currently running RHVoice process, if any. When
    /// speaking a new utterance we terminate the previous process.
    rhvoice_process: Option<Child>,
}

impl TtsEngine {
    /// Create a new TTS engine. Internally this initialises the system
    /// speech synthesis backend. If no backend is available on the host
    /// platform this will return an error.
    pub fn new() -> Result<Self> {
        // Check the VOICE_ENGINE environment variable. If set to
        // "rhvoice" (case insensitive), we'll use the rhvoice-test
        // command for synthesis. Otherwise fall back to the `tts` crate.
        let use_rhvoice = std::env::var("VOICE_ENGINE")
            .map(|v| v.to_lowercase() == "rhvoice")
            .unwrap_or(false);
        let tts = Tts::default().context("failed to initialise text‑to‑speech engine")?;
        Ok(Self {
            tts,
            use_rhvoice,
            rhvoice_process: None,
        })
    }

    /// Stop any ongoing speech, either internal TTS or external RHVoice process.
    pub async fn stop(&mut self) -> Result<()> {
        if self.use_rhvoice {
            if let Some(child) = self.rhvoice_process.as_mut() {
                let _ = child.kill().await;
            }
            self.rhvoice_process = None;
        } else {
            // Stop any ongoing utterances.
            self.tts.stop().map_err(|e| anyhow!(format!("Failed to stop TTS: {:?}", e)))?;
        }
        Ok(())
    }

    /// Choose a voice by name. The supplied name is matched case
    /// insensitively against the available voices. If a matching voice
    /// cannot be found the previous voice remains active and an error is
    /// returned.
    pub fn set_voice_by_name(&mut self, name: &str) -> Result<()> {
        // If using RHVoice CLI we cannot programmatically select a
        // voice through the `tts` crate. The CLI will use its default
        // voice or the voice specified via command line arguments. In
        // this case we ignore the requested voice and return Ok.
        if self.use_rhvoice {
            return Ok(());
        }
        let available = self.tts.voices().context("failed to enumerate voices")?;
        let target = name.to_lowercase();
        for voice in available {
            // Voice names are accessed via the `name()` method. Compare
            // case‑insensitively against the requested string.
            if voice.name().to_lowercase().contains(&target) {
                // `set_voice` expects a reference to the desired voice.
                self.tts
                    .set_voice(&voice)
                    .context("failed to set TTS voice")?;
                return Ok(());
            }
        }
        Err(anyhow!(format!("no voice matching '{name}' found")))
    }

    /// Speak the provided text. Existing speech will be interrupted if it
    /// is still playing. This method is asynchronous because the call to
    /// [`tts::Tts::speak`] blocks until the underlying OS has queued the
    /// utterance. We use `spawn_blocking` so as not to stall the Tokio
    /// executor while synthesis takes place.
    pub async fn speak(&mut self, text: &str) -> Result<()> {
        // If using RHVoice CLI, spawn an external process to speak.
        if self.use_rhvoice {
            // Terminate any existing process if it is still running.
            if let Some(child) = self.rhvoice_process.as_mut() {
                let _ = child.kill().await;
            }
            // Spawn the rhvoice-test process. We pass the "slt" voice by
            // default to approximate the Python implementation. You can
            // customise this by changing the argument or by setting
            // environment variables in the future.
            let mut cmd = Command::new("/snap/bin/rhvoice.test");
            cmd.arg("-p").arg("slt").stdin(std::process::Stdio::piped());
            let mut child = cmd.spawn().context("failed to spawn RHVoice process")?;
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                stdin
                    .write_all(text.as_bytes())
                    .await
                    .context("failed to write to RHVoice stdin")?;
                // Close stdin to let rhvoice know the input is complete.
                stdin.shutdown().await.ok();
            }
            // Store the handle so that cancellation can stop the process,
            // then await completion of the speech process.
            self.rhvoice_process = Some(child);
            if let Some(child) = self.rhvoice_process.as_mut() {
                let _ = child.wait().await;
            }
            self.rhvoice_process = None;
            return Ok(());
        }

        // Default path: use the built‑in TTS engine via the tts crate. We
        // clone the engine and speak on a blocking thread to avoid
        // stalling the async runtime.
        let text_owned = text.to_owned();
        let tts = self.tts.clone();
        tokio::task::spawn_blocking(move || {
            let mut tts = tts;
            // Stop any existing utterances. Ignore errors here since we
            // immediately follow with a new speak call.
            let _ = tts.stop();
            tts.speak(&text_owned, true)
                .map_err(|e| anyhow!(format!("TTS speak failed: {e:?}")))
        })
        .await
        .context("failed to join blocking TTS task")??;
        Ok(())
    }
}
