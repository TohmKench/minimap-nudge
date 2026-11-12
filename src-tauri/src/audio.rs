//! Sound decoding (symphonia) and playback (rodio) on a dedicated thread.
//!
//! - Files are fully decoded up front. Alert sounds are short, and decoding
//!   from an in-memory buffer avoids container quirks (M4A/OGG need seeking).
//! - The output device is opened per sound and released afterwards, so
//!   switching headsets mid-session just works and a missing device is harmless.
//! - Decoder panics are caught; a bad file can't take the app down.

use crate::logger::log;
use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink};
use std::fs;
use std::io::{Cursor, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Longest clip we'll decode (anything longer is truncated): 30 s.
const MAX_SECONDS: usize = 30;

#[derive(Debug, Clone)]
pub enum SoundSource {
    Builtin(&'static [u8]),
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Decoded {
    pub channels: u16,
    pub sample_rate: u32,
    /// Interleaved f32 samples.
    pub samples: Vec<f32>,
}

impl Decoded {
    pub fn duration_secs(&self) -> f64 {
        self.samples.len() as f64 / (self.channels.max(1) as f64 * self.sample_rate.max(1) as f64)
    }
}

/// Decode a whole sound file held in memory. `ext` is a format hint.
pub fn decode(bytes: Vec<u8>, ext: Option<&str>) -> Result<Decoded, String> {
    guarded(move || decode_inner(bytes, ext))
}

pub fn decode_file(path: &Path) -> Result<Decoded, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
    decode(bytes, ext.as_deref())
}

fn decode_inner(bytes: Vec<u8>, ext: Option<&str>) -> Result<Decoded, String> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("unrecognised format ({e})"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported codec ({e})"))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut spec = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) if !samples.is_empty() => {
                log(&format!("audio: stopped reading early: {e}"));
                break;
            }
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let s = *buf.spec();
                spec.get_or_insert(s);
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, s);
                sb.copy_interleaved_ref(buf);
                samples.extend_from_slice(sb.samples());
                let limit = MAX_SECONDS * s.rate as usize * s.channels.count().max(1);
                if samples.len() >= limit {
                    samples.truncate(limit);
                    break;
                }
            }
            Err(SymError::DecodeError(_)) => continue, // skip a corrupt frame
            Err(e) => return Err(e.to_string()),
        }
    }
    let spec = spec.ok_or("no audio could be decoded")?;
    if samples.is_empty() {
        return Err("no audio could be decoded".into());
    }
    Ok(Decoded { channels: spec.channels.count() as u16, sample_rate: spec.rate, samples })
}

/// Third-party decoders occasionally panic on odd files; treat that as an error.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err("decoder crashed on this file".into()))
}

// ---------------------------------------------------------------- playback

struct Play {
    source: SoundSource,
    volume: f32,
}

#[derive(Clone)]
pub struct AudioPlayer {
    tx: Sender<Play>,
}

impl AudioPlayer {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("audio".into())
            .spawn(move || audio_thread(rx))
            .expect("audio thread");
        Self { tx }
    }

    pub fn play(&self, source: SoundSource, volume: f64) {
        let _ = self.tx.send(Play { source, volume: volume.clamp(0.0, 1.0) as f32 });
    }
}

fn load(source: &SoundSource) -> Result<Decoded, String> {
    match source {
        SoundSource::Builtin(b) => decode(b.to_vec(), Some("wav")),
        SoundSource::File(p) => decode_file(p),
    }
}

/// Plays `job`; returns the next job if one arrived mid-sound (it interrupts).
fn play_one(job: Play, rx: &Receiver<Play>) -> Option<Play> {
    let sound = match load(&job.source) {
        Ok(s) => s,
        Err(e) => {
            log(&format!("audio: could not decode sound: {e}"));
            return None;
        }
    };
    let (_stream, handle) = match OutputStream::try_default() {
        Ok(s) => s,
        Err(e) => {
            log(&format!("audio: no output device: {e}"));
            return None;
        }
    };
    let sink = match Sink::try_new(&handle) {
        Ok(s) => s,
        Err(e) => {
            log(&format!("audio: could not open sink: {e}"));
            return None;
        }
    };
    sink.set_volume(job.volume);
    sink.append(SamplesBuffer::new(sound.channels, sound.sample_rate, sound.samples));
    // Wait for the sound to finish, but let a new request cut it short.
    loop {
        if sink.empty() {
            thread::sleep(Duration::from_millis(60)); // let the device drain the tail
            return None;
        }
        match rx.recv_timeout(Duration::from_millis(30)) {
            Ok(next) => {
                sink.stop();
                return Some(next);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

fn audio_thread(rx: Receiver<Play>) {
    while let Ok(mut job) = rx.recv() {
        loop {
            // Never let one bad sound kill the audio thread.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| play_one(job, &rx))) {
                Ok(Some(next)) => job = next,
                Ok(None) => break,
                Err(_) => {
                    log("audio: playback panicked; skipped");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::BUILTIN_SOUNDS;

    fn sample(ext: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../tests/audio/sample.{ext}"))
    }

    #[test]
    fn builtin_sounds_decode() {
        for s in BUILTIN_SOUNDS {
            let d = decode(s.bytes.to_vec(), Some("wav")).unwrap_or_else(|e| panic!("{}: {e}", s.id));
            assert!(d.duration_secs() > 0.3 && d.duration_secs() < 3.0, "{}: {}s", s.id, d.duration_secs());
        }
    }

    /// Generated by scripts/make-test-audio.sh (ffmpeg) from the built-in ping.
    #[test]
    fn upload_formats_decode() {
        for ext in ["mp3", "ogg", "flac", "m4a", "wav"] {
            let d = decode_file(&sample(ext)).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert!(d.duration_secs() > 1.0, "{ext} only {}s", d.duration_secs());
            assert!(d.samples.iter().any(|s| s.abs() > 0.1), "{ext} decoded to silence");
        }
    }

    #[test]
    fn decodes_without_extension_hint() {
        for ext in ["mp3", "ogg", "flac", "m4a"] {
            let bytes = fs::read(sample(ext)).unwrap();
            assert!(decode(bytes, None).is_ok(), "{ext} without hint");
        }
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        for ext in ["mp3", "ogg", "flac", "m4a", "wav"] {
            let p = dir.path().join(format!("junk.{ext}"));
            fs::write(&p, b"definitely not audio, just some bytes pretending to be a sound file").unwrap();
            assert!(decode_file(&p).is_err(), "{ext} junk should be rejected");
            // truncated real files: Ok or Err are both fine, panicking is not
            let real = fs::read(sample(ext)).unwrap();
            for cut in [16, 64, 512, real.len() / 2] {
                fs::write(&p, &real[..cut.min(real.len())]).unwrap();
                let _ = decode_file(&p);
            }
        }
    }

    #[test]
    fn playback_without_device_is_graceful() {
        let p = AudioPlayer::spawn();
        p.play(SoundSource::Builtin(BUILTIN_SOUNDS[0].bytes), 0.0);
        p.play(SoundSource::File(PathBuf::from("/nope/missing.mp3")), 0.5);
        thread::sleep(Duration::from_millis(300));
        p.play(SoundSource::Builtin(BUILTIN_SOUNDS[1].bytes), 0.0);
        thread::sleep(Duration::from_millis(100));
        assert!(p.tx.send(Play { source: SoundSource::Builtin(BUILTIN_SOUNDS[2].bytes), volume: 0.0 }).is_ok(), "audio thread still alive");
    }
}
