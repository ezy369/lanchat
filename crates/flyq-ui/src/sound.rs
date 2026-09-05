//! Notification sound playback.
//!
//! Plays a short sine-wave tone through the default audio output when a
//! notification-worthy event occurs (incoming message, file offer, file
//! complete). The sound is played on a detached OS thread so it never blocks
//! the GPUI event loop.
//!
//! Failures are swallowed and logged at debug level — a missing or busy audio
//! device must never disrupt the application.

use rodio::{OutputStream, Sink, Source};
use std::time::Duration;

/// Play a short notification tone (523 Hz, ~200 ms) on the default audio
/// output. The call returns immediately; the sound plays asynchronously on a
/// background thread.
pub fn play_notification() {
    std::thread::spawn(|| {
        let stream = match OutputStream::try_default() {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!("Failed to open audio output: {}", e);
                return;
            }
        };
        let (_stream, stream_handle) = (stream.0, stream.1);
        let sink = match Sink::try_new(&stream_handle) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!("Failed to create audio sink: {}", e);
                return;
            }
        };

        let source = rodio::source::SineWave::new(523.0)
            .take_duration(Duration::from_millis(200))
            .amplify(0.3);

        sink.append(source);
        // Block until the tone finishes, keeping the stream alive.
        sink.sleep_until_end();
    });
}
