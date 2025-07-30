//! Speech recognition support using the [`vosk`] crate and [`cpal`] for audio input.
//!
//! The [`SpeechRecognizer`] struct encapsulates a loaded Vosk model and a
//! selected microphone. It provides a simple blocking API for capturing a
//! short audio clip and converting it into text. Under the hood it uses
//! [`cpal`] to stream audio samples from the chosen device, down-mixes
//! multichannel input to mono and feeds the resulting `i16` samples into
//! a Vosk recogniser. Once recording is finished the recogniser is asked
//! for a final result and the transcript is returned.
//!
//! The environment variables `MIC_INDEX` and `MIC_NAME_KEYWORD` control
//! which microphone is selected at construction time. If `MIC_INDEX` is
//! provided and can be parsed as a `usize` then the device at that index
//! in the enumeration of available input devices is chosen. Otherwise, if
//! `MIC_NAME_KEYWORD` is set the first device whose name contains the
//! provided keyword (case insensitive) is used. If neither variable is
//! set or no match is found, the default input device is used. If there
//! is no default device the constructor returns an error.

use std::env;
use std::sync::mpsc::{self};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use vosk::{Model, Recognizer};

/// A simple wrapper around Vosk for capturing a short phrase from the microphone
/// and converting it to text.
pub struct SpeechRecognizer {
    model: Model,
    device: cpal::Device,
}

impl SpeechRecognizer {
    /// Create a new speech recogniser from the given model path. This will
    /// attempt to load the Vosk model from `model_path` and select a
    /// microphone based on environment variables.
    pub fn new(model_path: &str) -> Result<Self> {
        // Load the Vosk model from disk. If the model files cannot be found
        // or are incompatible with the host platform Vosk will return an
        // error here. See the crate documentation for setup instructions.
        let model = Model::new(model_path)
            .with_context(|| format!("Failed to load Vosk model from '{}'.", model_path))?;

        // Discover the audio input devices available on this system.
        let host = cpal::default_host();
        let device_iter = host
            .input_devices()
            .with_context(|| "Failed to enumerate input audio devices")?;
        // Collect devices into a vector because the iterator cannot be cloned.
        let devices: Vec<cpal::Device> = device_iter.collect();

        // Try to select a device based on MIC_INDEX or MIC_NAME_KEYWORD. Both
        // variables are optional; if neither is provided we fall back to the
        // default input device. If parsing fails or no matching device is
        // found the default device will also be used.
        let mic_index = env::var("MIC_INDEX")
            .ok()
            .and_then(|s| s.parse::<usize>().ok());
        let mic_keyword = env::var("MIC_NAME_KEYWORD").ok();

        let mut selected_device: Option<cpal::Device> = None;

        if let Some(idx) = mic_index {
            if idx < devices.len() {
                selected_device = Some(devices[idx].clone());
            }
        }

        if selected_device.is_none() {
            if let Some(keyword) = mic_keyword.clone() {
                let keyword_lower = keyword.to_lowercase();
                for dev in &devices {
                    if let Ok(name) = dev.name() {
                        if name.to_lowercase().contains(&keyword_lower) {
                            selected_device = Some(dev.clone());
                            break;
                        }
                    }
                }
            }
        }

        // Fall back to default input device if none selected yet
        if selected_device.is_none() {
            selected_device = host.default_input_device();
        }

        let device = selected_device.ok_or_else(|| anyhow!("No input audio device found"))?;

        if let Ok(name) = device.name() {
            log::info!("Using microphone: {}", name);
        }

        Ok(Self { model, device })
    }

    /// Listen to the microphone for a fixed duration and return the recognised
    /// transcript. If no speech is detected an empty string is returned. Any
    /// errors encountered during recording or recognition will be returned to
    /// the caller.
    pub fn listen_for_phrase(&self, duration: Duration) -> Result<String> {
        // Obtain the default input configuration. This contains the sample rate,
        // number of channels and sample format supported by the device. If the
        // device does not support input we return an error.
        let config = self
            .device
            .default_input_config()
            .with_context(|| "Failed to get default input configuration")?;

        // We'll build a recogniser for the detected sample rate. Vosk expects
        // sample rates as floating point values.
        let sample_rate = config.sample_rate().0 as f32;
        let channels = config.channels() as usize;
        let mut recogniser = Recognizer::new(&self.model, sample_rate)
            .with_context(|| "Failed to create Vosk recogniser")?;

        // We do not need word-level timing or alternatives for the simple
        // phrase recognition use case.
        recogniser.set_words(false);
        recogniser.set_max_alternatives(0);

        // Create a channel to transfer audio samples from the CPAL callback to
        // our consumer thread. We use a standard synchronous channel from
        // std::sync to avoid pulling in additional async dependencies here.
        let (tx, rx) = mpsc::channel::<Vec<i16>>();
        let tx_err = tx.clone();

        // Define an error callback for CPAL. If anything goes wrong while
        // streaming CPAL will call this closure. We simply log the error.
        let err_fn = |err| {
            log::error!("An error occurred on the input audio stream: {}", err);
        };

        // Build the input stream according to the detected sample format. Each
        // closure converts the raw input buffer into a vector of i16 samples
        // representing the mono audio stream and then sends it over the
        // channel. Channels are interleaved so we take only the first sample
        // from each frame to reduce to mono. If sending fails (because the
        // receiver has been dropped) the callback simply returns.
        let stream: cpal::Stream = match config.sample_format() {
            SampleFormat::I16 => {
                let tx = tx.clone();
                self.device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let mut mono = Vec::with_capacity(data.len() / channels);
                        for frame in data.chunks(channels) {
                            mono.push(frame[0]);
                        }
                        if tx.send(mono).is_err() {
                            // Receiver has been dropped; stop sending
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            SampleFormat::U16 => {
                let tx = tx.clone();
                self.device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], _| {
                        let mut mono = Vec::with_capacity(data.len() / channels);
                        for frame in data.chunks(channels) {
                            // Convert unsigned sample to signed range by subtracting midpoint
                            let s = frame[0] as i32 - 32768;
                            mono.push(s as i16);
                        }
                        if tx.send(mono).is_err() {
                            // Receiver has been dropped
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            SampleFormat::F32 => {
                let tx = tx.clone();
                self.device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| {
                        let mut mono = Vec::with_capacity(data.len() / channels);
                        for frame in data.chunks(channels) {
                            // Convert from [-1.0, 1.0] float to i16 range
                            let sample = frame[0];
                            let s = (sample * 32768.0).clamp(-32768.0, 32767.0) as i16;
                            mono.push(s);
                        }
                        if tx.send(mono).is_err() {
                            // Receiver has been dropped
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            // Handle any other sample formats by returning an error. cpal
            // marks SampleFormat as non‑exhaustive so we must include a
            // wildcard arm. If a new format becomes available you can
            // extend the match accordingly.
            _ => {
                return Err(anyhow!(format!(
                    "Unsupported sample format: {:?}",
                    config.sample_format()
                )));
            }
        };

        // Start streaming from the microphone
        stream
            .play()
            .with_context(|| "Failed to start audio input stream")?;

        let start_time = Instant::now();
        let mut samples: Vec<i16> = Vec::new();
        // We'll stop recording early if we detect a period of silence after
        // initial speech. Define a simple amplitude threshold and a
        // silence timeout. When audio levels remain below the threshold
        // for `silence_timeout` after speech has started, we break out.
        let silence_threshold: i16 = 500;
        let silence_timeout = Duration::from_millis(800);
        let min_capture_time = Duration::from_millis(1000);
        let mut last_speech = Instant::now();
        let mut speech_started = false;
        // Pull chunks off the channel until the timeout expires. We use a
        // short recv_timeout to periodically check for elapsed time and
        // update our silence detection logic.
        while start_time.elapsed() < duration {
            let timeout = duration
                .checked_sub(start_time.elapsed())
                .unwrap_or_else(|| Duration::from_millis(0));
            match rx.recv_timeout(timeout) {
                Ok(chunk) => {
                    // Append the samples to our buffer
                    samples.extend_from_slice(&chunk);
                    // Determine if this chunk contains speech by checking
                    // if any sample exceeds the threshold.
                    let has_speech = chunk.iter().any(|s| s.wrapping_abs() > silence_threshold);
                    if has_speech {
                        speech_started = true;
                        last_speech = Instant::now();
                    }
                    // If we've already captured at least `min_capture_time` of audio
                    // and we've not heard speech for `silence_timeout`, break early.
                    if speech_started
                        && start_time.elapsed() > min_capture_time
                        && last_speech.elapsed() > silence_timeout
                    {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Timeout elapsed; break from loop
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }

        // Stop and drop the stream. Dropping the stream closes the input.
        drop(stream);
        drop(tx_err);

        // If we captured any audio, feed it into the recogniser and fetch the
        // final result. Otherwise return an empty string.
        if !samples.is_empty() {
            recogniser.accept_waveform(&samples)?;
            let final_result = recogniser.final_result();
            // `single()` returns `Option<CompleteResultSingle>`; extract
            // the recognised transcript if present.
            if let Some(single) = final_result.single() {
                return Ok(single.text.to_string());
            }
        }
        Ok(String::new())
    }
}
