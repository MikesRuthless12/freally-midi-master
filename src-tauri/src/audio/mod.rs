//! Playback (TASK-027, FR-014).
//!
//! Three threads, and which one does what is the whole design:
//!
//! - **The UI thread** answers commands. It builds schedules, which allocates,
//!   and hands them over a lock-free ring.
//! - **The audio thread** is cpal's callback. It reads commands, mixes voices
//!   and limits the result. It never allocates, never locks, never touches the
//!   filesystem, and never frees — a replaced schedule is handed *back* over a
//!   second ring for the UI thread to drop.
//! - **The publisher thread** ticks at 30 Hz: it drains retired schedules and
//!   emits the playhead and any device news to the frontend.
//!
//! **A lost device is recovered from, not merely reported** (FR-014). cpal's
//! error callback marks it gone; the device thread then drops the dead stream,
//! builds new rings, and opens another — retrying for as long as it takes,
//! because the alternative is an app that is silently deaf until relaunched.
//! Recovery is a *replacement*: the old engine owns one end of each ring and
//! dies with the stream, so [`Channels`] exists to swap the UI's ends
//! underneath it without any command site knowing. Playback does not resume by
//! itself — audio restarting on its own some seconds after an unplug is
//! startling rather than helpful.
//!
//! [`Engine::process`] is a pure function of its inputs, so the tests drive the
//! whole sequencer and mixer with no device present — which is what makes any
//! of this testable in CI.

pub mod kit;
pub mod sampler;
pub mod transport;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use engine::pattern::Pattern;
use serde::Serialize;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};
use ts_rs::TS;

use kit::Kit;
use sampler::Sampler;
use transport::{Schedule, Transport};

/// How many commands may be in flight. Playback commands arrive at the speed a
/// person can click, so this is generous by orders of magnitude.
const RING_CAPACITY: usize = 32;

/// The playhead is for drawing, not for timing. 30 Hz is smooth to the eye and
/// costs the frontend a fraction of a frame (FR-014).
const PUBLISH_INTERVAL: Duration = Duration::from_millis(33);

/// What the UI asks the audio thread to do.
///
/// `Play` carries a boxed schedule so installing it moves a pointer rather than
/// copying a note list on the audio thread.
enum Command {
    Play(Box<Schedule>),
    Stop,
    SetLooping(bool),
    Preview {
        pad: usize,
        velocity: f32,
        semis: f32,
    },
}

/// What the output device is doing, for the UI to say so.
///
/// A `u8` because it lives in an atomic beside the rest of the shared state;
/// the conversion is one match rather than a dependency on a crate that would
/// derive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    Ok,
    /// Gone, and a rebuild is being attempted.
    Recovering,
    /// Gone, and rebuilding failed. Retried until it works.
    Failed,
}

impl DeviceState {
    fn as_u8(self) -> u8 {
        match self {
            DeviceState::Ok => 0,
            DeviceState::Recovering => 1,
            DeviceState::Failed => 2,
        }
    }

    fn from_u8(value: u8) -> DeviceState {
        match value {
            1 => DeviceState::Recovering,
            2 => DeviceState::Failed,
            _ => DeviceState::Ok,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            DeviceState::Ok => "ok",
            DeviceState::Recovering => "recovering",
            DeviceState::Failed => "failed",
        }
    }
}

/// How often the device thread looks for a lost device, and how long it waits
/// between failed rebuilds.
///
/// A device usually comes back within a second or two of being replugged, and
/// a user switching interfaces expects it to just work rather than to have to
/// restart. Retrying forever is right: the alternative is an app that is
/// silently deaf until relaunched.
const DEVICE_POLL: Duration = Duration::from_millis(250);
const RECOVERY_BACKOFF: Duration = Duration::from_millis(1_000);

/// State shared between the threads, all of it lock-free.
struct Shared {
    playing: AtomicBool,
    /// Position within the loop, in samples.
    position: AtomicU64,
    loop_len: AtomicU64,
    /// Beats in one cycle, so the frontend can turn samples into bars.
    beats: AtomicU64,
    /// Set by cpal's error callback; the device thread rebuilds on it.
    device_lost: AtomicBool,
    /// A [`DeviceState`] discriminant, for the UI to report.
    device_state: AtomicU8,
    /// Bumped every time a stream is successfully opened. The publisher watches
    /// it so a recovery is announced once, not on every poll.
    device_generation: AtomicU64,
    sample_rate: AtomicU64,
}

impl Shared {
    fn new() -> Self {
        Shared {
            playing: AtomicBool::new(false),
            position: AtomicU64::new(0),
            loop_len: AtomicU64::new(0),
            beats: AtomicU64::new(0),
            device_lost: AtomicBool::new(false),
            device_state: AtomicU8::new(DeviceState::Ok.as_u8()),
            device_generation: AtomicU64::new(0),
            sample_rate: AtomicU64::new(0),
        }
    }

    fn device_state(&self) -> DeviceState {
        DeviceState::from_u8(self.device_state.load(Ordering::Relaxed))
    }

    fn set_device_state(&self, state: DeviceState) {
        self.device_state.store(state.as_u8(), Ordering::Relaxed);
    }
}

/// The audio-thread half: everything the callback touches.
struct Engine {
    kit: Arc<Kit>,
    sampler: Sampler,
    transport: Transport,
    commands: rtrb::Consumer<Command>,
    retire: rtrb::Producer<Box<Schedule>>,
    /// A schedule waiting for room in the retire ring.
    ///
    /// Belt and braces: the UI thread drains the ring before every push, so it
    /// cannot realistically be full. If it ever were, holding the box here
    /// until next block is still correct — dropping it would free memory on the
    /// audio thread, which is the one thing that must never happen.
    pending_retire: Option<Box<Schedule>>,
    shared: Arc<Shared>,
    sample_rate: f64,
    channels: usize,
}

impl Engine {
    /// Fill one buffer. This is the audio callback.
    fn process(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        self.drain_commands();
        self.retry_retire();

        let channels = self.channels.max(1);
        let frames = out.len() / channels;
        let mut done = 0usize;

        // Render in segments so each hit starts on its own sample rather than
        // at the head of whichever buffer it landed in.
        while done < frames {
            let segment = self.transport.next_segment(frames - done);

            if let Some(trigger) = segment.trigger {
                self.sampler.trigger(
                    &self.kit,
                    trigger.pad,
                    trigger.velocity,
                    trigger.semis,
                    self.sample_rate,
                );
            }

            if segment.frames == 0 {
                if segment.trigger.is_none() {
                    // Nothing more to fire and nothing to wait for.
                    break;
                }
                continue;
            }

            let start = done * channels;
            let end = (done + segment.frames) * channels;
            self.sampler
                .render(&self.kit, &mut out[start..end], channels);
            self.transport.advance(segment.frames);
            done += segment.frames;
        }

        // Whatever is left is the tail of voices still ringing after the
        // transport stopped — a stop must not chop the decay dead.
        if done < frames {
            let start = done * channels;
            self.sampler.render(&self.kit, &mut out[start..], channels);
        }

        sampler::limit(out);

        self.shared
            .playing
            .store(self.transport.playing(), Ordering::Relaxed);
        self.shared
            .position
            .store(self.transport.position(), Ordering::Relaxed);
        self.shared
            .loop_len
            .store(self.transport.loop_len(), Ordering::Relaxed);
    }

    fn drain_commands(&mut self) {
        while let Ok(command) = self.commands.pop() {
            match command {
                Command::Play(schedule) => {
                    self.shared
                        .beats
                        .store(schedule.beats.to_bits(), Ordering::Relaxed);
                    self.sampler.stop_all();
                    if let Some(old) = self.transport.play(schedule) {
                        self.push_retire(old);
                    }
                }
                Command::Stop => {
                    self.transport.stop();
                    // Voices are left to ring out; the transport stopping is
                    // not the same as the sound stopping.
                }
                Command::SetLooping(looping) => self.transport.set_looping(looping),
                Command::Preview {
                    pad,
                    velocity,
                    semis,
                } => {
                    self.sampler
                        .trigger(&self.kit, pad, velocity, semis, self.sample_rate);
                }
            }
        }
    }

    fn push_retire(&mut self, schedule: Box<Schedule>) {
        match self.retire.push(schedule) {
            Ok(()) => {}
            Err(rtrb::PushError::Full(schedule)) => self.pending_retire = Some(schedule),
        }
    }

    fn retry_retire(&mut self) {
        if let Some(schedule) = self.pending_retire.take() {
            self.push_retire(schedule);
        }
    }
}

/// The UI's ends of the two rings, swappable underneath it.
///
/// Held behind an `Arc` by both the app and the device thread, because
/// recovering from a lost device means building a *new* engine — and the old
/// engine owns the other ends of the old rings, which die with it. Rebuilding
/// therefore has to replace what the UI pushes into, and this is the seam that
/// lets it, without any command site knowing a swap ever happened.
struct Channels {
    commands: Mutex<rtrb::Producer<Command>>,
    retired: Mutex<rtrb::Consumer<Box<Schedule>>>,
}

impl Channels {
    /// A fresh pair. Returns the UI's ends, then the audio thread's.
    fn new() -> (
        Channels,
        rtrb::Consumer<Command>,
        rtrb::Producer<Box<Schedule>>,
    ) {
        let (commands_tx, commands_rx) = rtrb::RingBuffer::new(RING_CAPACITY);
        let (retire_tx, retire_rx) = rtrb::RingBuffer::new(RING_CAPACITY);
        (
            Channels {
                commands: Mutex::new(commands_tx),
                retired: Mutex::new(retire_rx),
            },
            commands_rx,
            retire_tx,
        )
    }

    /// Replace both rings, handing back the ends for a new engine.
    ///
    /// Anything still queued in the old command ring is discarded with it, and
    /// that is correct: those commands were addressed to a device that no
    /// longer exists, and replaying a "play" into a freshly opened stream would
    /// start audio the user never asked for after an unplug.
    fn rebuild(&self) -> (rtrb::Consumer<Command>, rtrb::Producer<Box<Schedule>>) {
        let (commands_tx, commands_rx) = rtrb::RingBuffer::new(RING_CAPACITY);
        let (retire_tx, retire_rx) = rtrb::RingBuffer::new(RING_CAPACITY);
        if let Ok(mut commands) = self.commands.lock() {
            *commands = commands_tx;
        }
        if let Ok(mut retired) = self.retired.lock() {
            *retired = retire_rx;
        }
        (commands_rx, retire_tx)
    }
}

/// The app-facing handle, held as Tauri state.
pub struct Audio {
    channels: Arc<Channels>,
    shared: Arc<Shared>,
    kit: Arc<Kit>,
    /// Why playback is unavailable, if it is. Held rather than logged: a user
    /// pressing play deserves the reason, not silence.
    pub failure: Option<String>,
}

impl Audio {
    /// Load the kit and open the output device.
    ///
    /// Never fails the launch. A machine with no sound card still runs the app,
    /// generates and exports; only playback is unavailable, and `failure` says
    /// why.
    pub fn start(kit_dir: &std::path::Path) -> Audio {
        let shared = Arc::new(Shared::new());
        let (channels, commands_rx, retire_tx) = Channels::new();
        let channels = Arc::new(channels);

        let kit = match Kit::load(kit_dir) {
            Ok(kit) => Arc::new(kit),
            Err(e) => return Audio::unavailable(e),
        };

        let failure = spawn_stream(
            Arc::clone(&kit),
            Arc::clone(&channels),
            commands_rx,
            retire_tx,
            Arc::clone(&shared),
        )
        .err();

        Audio {
            channels,
            shared,
            kit,
            failure,
        }
    }

    /// An engine that cannot play, carrying the reason.
    ///
    /// The rings and the empty kit exist so every command still has something
    /// well-formed to refuse against — a `None` engine would mean an
    /// `Option` check at every call site instead of one message.
    fn unavailable(reason: String) -> Audio {
        Audio {
            channels: Arc::new(Channels::new().0),
            shared: Arc::new(Shared::new()),
            kit: Arc::new(Kit {
                id: "none".into(),
                pads: Vec::new(),
            }),
            failure: Some(reason),
        }
    }

    /// The device's sample rate, once the stream is open.
    fn sample_rate(&self) -> f64 {
        let rate = self.shared.sample_rate.load(Ordering::Relaxed);
        if rate == 0 {
            44_100.0
        } else {
            rate as f64
        }
    }

    /// Drop anything the audio thread handed back, then send a command.
    ///
    /// Draining first is what guarantees the retire ring always has room, so
    /// the audio thread never has to hold a schedule it cannot give away.
    fn send(&self, command: Command) -> Result<(), String> {
        if let Ok(mut retired) = self.channels.retired.lock() {
            while retired.pop().is_ok() {}
        }
        self.channels
            .commands
            .lock()
            .map_err(|_| "the audio channel is poisoned".to_string())?
            .push(command)
            .map_err(|_| "the audio thread is not keeping up".to_string())
    }
}

/// Open the device and hand the callback to a thread that owns it.
///
/// cpal's `Stream` is not `Send` everywhere, so it is built on the thread that
/// will keep it alive, and that thread parks for the life of the process.
fn spawn_stream(
    kit: Arc<Kit>,
    channels: Arc<Channels>,
    commands: rtrb::Consumer<Command>,
    retire: rtrb::Producer<Box<Schedule>>,
    shared: Arc<Shared>,
) -> Result<(), String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("audio-device".into())
        .spawn(move || {
            let built = build_stream(Arc::clone(&kit), commands, retire, Arc::clone(&shared));
            match built {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    shared.device_generation.fetch_add(1, Ordering::Relaxed);
                    // A cpal stream stops the instant it is dropped, so this
                    // thread exists to hold it — and, when the device goes
                    // away, to drop it deliberately and open another.
                    tend_device(stream, &kit, &channels, &shared, |kit, rx, tx, shared| {
                        build_stream(kit, rx, tx, shared)
                    });
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        })
        .map_err(|e| format!("could not start the audio thread: {e}"))?;

    ready_rx
        .recv()
        .map_err(|_| "the audio thread stopped before it opened a device".to_string())?
}

/// Hold the stream, and rebuild it when the device disappears (FR-014).
///
/// Recovery is a *replacement*, not a repair: the old engine owns one end of
/// each ring and dies with the stream, so new rings are made and swapped in
/// underneath the UI. Playback does not resume by itself — the schedule went
/// with the old engine, and audio restarting on its own after an unplug is
/// startling rather than helpful. Pressing play works again, which is the
/// thing that was actually broken.
///
/// Generic over the builder so the state machine can be tested without a sound
/// card; nothing else here can be.
fn tend_device<S>(
    stream: S,
    kit: &Arc<Kit>,
    channels: &Arc<Channels>,
    shared: &Arc<Shared>,
    mut build: impl FnMut(
        Arc<Kit>,
        rtrb::Consumer<Command>,
        rtrb::Producer<Box<Schedule>>,
        Arc<Shared>,
    ) -> Result<S, String>,
) {
    let mut held = Some(stream);

    loop {
        // Holding nothing means the last rebuild failed, and a device being
        // plugged back in is worth waiting a beat for rather than hammering.
        std::thread::sleep(if held.is_none() {
            RECOVERY_BACKOFF
        } else {
            DEVICE_POLL
        });

        let lost = shared.device_lost.swap(false, Ordering::Relaxed);
        // Two reasons to rebuild: the device just went away, or it went away
        // earlier and every attempt since has failed.
        if !lost && held.is_some() {
            continue;
        }

        shared.playing.store(false, Ordering::Relaxed);
        shared.set_device_state(DeviceState::Recovering);

        // Drop the dead stream *before* opening another. Holding both means
        // asking the OS for a second stream on a device it is still cleaning
        // up after, which is how a recovery attempt fails for a reason that
        // has nothing to do with the device being gone.
        held = None;

        let (commands, retire) = channels.rebuild();
        match build(Arc::clone(kit), commands, retire, Arc::clone(shared)) {
            Ok(stream) => {
                held = Some(stream);
                shared.set_device_state(DeviceState::Ok);
                shared.device_generation.fetch_add(1, Ordering::Relaxed);
                eprintln!("audio: device recovered");
            }
            Err(e) => {
                shared.set_device_state(DeviceState::Failed);
                eprintln!("audio: could not reopen a device: {e}");
            }
        }
    }
}

fn build_stream(
    kit: Arc<Kit>,
    commands: rtrb::Consumer<Command>,
    retire: rtrb::Producer<Box<Schedule>>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("there is no audio output device")?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("the audio device has no usable output format: {e}"))?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    shared
        .sample_rate
        .store(u64::from(sample_rate), Ordering::Relaxed);

    let mut engine = Engine {
        kit,
        sampler: Sampler::default(),
        transport: Transport::default(),
        commands,
        retire,
        pending_retire: None,
        shared: Arc::clone(&shared),
        sample_rate: f64::from(sample_rate),
        channels,
    };

    let mut stream_config: cpal::StreamConfig = config.clone().into();
    // A small buffer is what keeps trigger latency under 10 ms; 256 frames is
    // 5.3 ms at 48 kHz. The device may refuse and pick its own, which is why
    // nothing downstream assumes a size.
    stream_config.buffer_size = cpal::BufferSize::Fixed(256);

    let error_shared = Arc::clone(&shared);
    let on_error = move |e| {
        eprintln!("audio: stream error: {e}");
        error_shared.device_lost.store(true, Ordering::Relaxed);
    };

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &stream_config,
            move |out: &mut [f32], _| engine.process(out),
            on_error,
            None,
        ),
        other => {
            return Err(format!(
                "the audio device wants {other} samples, which is not supported yet"
            ))
        }
    }
    .map_err(|e| format!("could not open the audio device: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("could not start playback: {e}"))?;
    Ok(stream)
}

/// What the frontend draws the playhead from.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-audio-types.ts")]
pub struct Playhead {
    pub playing: bool,
    /// Position through the loop, 0–1. Absent a loop, 0.
    pub position: f64,
    /// The same position in beats, for a bar counter.
    pub beat: f64,
}

/// Load the bundled kit and open a device.
///
/// The kit lives beside the dataset in the bundled `data/` resource, so it is
/// found the same way — and, like the dataset, a failure here is carried rather
/// than thrown: the app still launches.
pub fn start(app: &AppHandle) -> Audio {
    match app
        .path()
        .resolve("data/kits/trap-default", BaseDirectory::Resource)
    {
        Ok(dir) => {
            let audio = Audio::start(&dir);
            if let Some(failure) = &audio.failure {
                eprintln!("audio: unavailable — {failure}");
            }
            audio
        }
        Err(e) => Audio::unavailable(format!("could not locate the bundled kit: {e}")),
    }
}

/// What the UI needs to say about the output device (FR-014).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-audio-types.ts")]
pub struct DeviceNotice {
    /// `ok`, `recovering` or `failed`.
    pub state: String,
    /// True when this is a recovery rather than the first device opening, so
    /// the UI can say "back" instead of announcing something that never broke.
    pub recovered: bool,
}

/// Emit the playhead at 30 Hz and drop whatever the audio thread retired.
pub fn spawn_publisher(app: AppHandle) {
    std::thread::Builder::new()
        .name("audio-publish".into())
        .spawn(move || {
            let mut was_playing = false;
            let mut last_device = DeviceState::Ok;
            let mut last_generation = 0u64;
            loop {
                std::thread::sleep(PUBLISH_INTERVAL);
                let Some(audio) = app.try_state::<Audio>() else {
                    continue;
                };

                if let Ok(mut retired) = audio.channels.retired.lock() {
                    while retired.pop().is_ok() {}
                }

                // Device news, on change only — a poll loop that emitted every
                // tick would put a toast on screen thirty times a second.
                let device = audio.shared.device_state();
                let generation = audio.shared.device_generation.load(Ordering::Relaxed);
                if device != last_device || generation != last_generation {
                    // The first stream opening is not a recovery, and saying
                    // "your device is back" at launch would be nonsense.
                    let recovered = device == DeviceState::Ok && last_generation > 0;
                    if device != DeviceState::Ok || recovered {
                        let _ = app.emit(
                            "playback:device",
                            DeviceNotice {
                                state: device.as_str().to_string(),
                                recovered,
                            },
                        );
                    }
                    last_device = device;
                    last_generation = generation;
                }

                let playing = audio.shared.playing.load(Ordering::Relaxed);
                // One last event after a stop, so the playhead does not freeze
                // wherever it happened to be when the transport ended.
                if !playing && !was_playing {
                    continue;
                }
                was_playing = playing;

                let _ = app.emit("playback:playhead", playhead(&audio));
            }
        })
        .ok();
}

fn playhead(audio: &Audio) -> Playhead {
    let loop_len = audio.shared.loop_len.load(Ordering::Relaxed);
    let position = audio.shared.position.load(Ordering::Relaxed);
    let beats = f64::from_bits(audio.shared.beats.load(Ordering::Relaxed));
    let fraction = if loop_len == 0 {
        0.0
    } else {
        position as f64 / loop_len as f64
    };

    Playhead {
        playing: audio.shared.playing.load(Ordering::Relaxed),
        position: fraction,
        beat: if beats.is_finite() {
            fraction * beats
        } else {
            0.0
        },
    }
}

/// What `play_pattern` reports back.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/lib/ipc-audio-types.ts")]
pub struct PlaybackStarted {
    /// Notes in lanes this kit has no pad for. Zero on the shipped kit; a
    /// number here means part of the pattern is inaudible.
    pub unplaced_notes: usize,
    pub voices: usize,
}

/// The body of `play_pattern`, separated from the command so the refusal paths
/// can be tested without a Tauri app or a sound card.
fn play_pattern_inner(
    audio: &Audio,
    pattern: &Pattern,
    looping: Option<bool>,
) -> Result<PlaybackStarted, String> {
    if let Some(failure) = &audio.failure {
        return Err(failure.clone());
    }

    let schedule = transport::schedule(pattern, &audio.kit, audio.sample_rate());
    let unplaced = schedule.unplaced;
    let voices = schedule.triggers.len();

    audio.send(Command::SetLooping(looping.unwrap_or(true)))?;
    audio.send(Command::Play(Box::new(schedule)))?;

    Ok(PlaybackStarted {
        unplaced_notes: unplaced,
        voices,
    })
}

fn stop_inner(audio: &Audio) -> Result<(), String> {
    if audio.failure.is_some() {
        // Stopping something that never started is not an error worth showing.
        return Ok(());
    }
    audio.send(Command::Stop)
}

#[tauri::command]
pub fn play_pattern(
    pattern: Pattern,
    looping: Option<bool>,
    audio: State<'_, Audio>,
) -> Result<PlaybackStarted, String> {
    play_pattern_inner(&audio, &pattern, looping)
}

#[tauri::command]
pub fn stop_playback(audio: State<'_, Audio>) -> Result<(), String> {
    stop_inner(&audio)
}

#[tauri::command]
pub fn set_looping(looping: bool, audio: State<'_, Audio>) -> Result<(), String> {
    if audio.failure.is_some() {
        return Ok(());
    }
    audio.send(Command::SetLooping(looping))
}

/// Audition one pad, for the kit UI and for checking a device is alive.
#[tauri::command]
pub fn preview_pad(
    pad: String,
    velocity: Option<f32>,
    audio: State<'_, Audio>,
) -> Result<(), String> {
    if let Some(failure) = &audio.failure {
        return Err(failure.clone());
    }
    let index = audio
        .kit
        .pads
        .iter()
        .position(|p| p.id == pad)
        .ok_or_else(|| format!("this kit has no pad called `{pad}`"))?;

    audio.send(Command::Preview {
        pad: index,
        velocity: velocity.unwrap_or(1.0).clamp(0.0, 1.0),
        semis: 0.0,
    })
}

/// Whether playback is available, and why not if it is not.
#[tauri::command]
pub fn playback_status(audio: State<'_, Audio>) -> Option<String> {
    audio.failure.clone()
}

/// An allocator that counts, so a test can prove the audio callback does not
/// allocate (FR-014: "no allocation or locks on the callback").
///
/// This is the only way to assert that property. Reading the code and
/// concluding "it looks allocation-free" is exactly how an allocation gets in —
/// a `Vec` that grows, a `format!` in an error path, a `Box` dropped rather
/// than retired. Any of those can block on the allocator's lock while the
/// device waits, and the result is a click the developer never hears because it
/// only happens under memory pressure on someone else's machine.
///
/// Counting is thread-local and off by default, so the rest of the suite runs
/// at full speed and concurrent tests cannot see each other's allocations.
#[cfg(test)]
mod counting_allocator {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static WATCHING: Cell<bool> = const { Cell::new(false) };
        static COUNT: Cell<usize> = const { Cell::new(0) };
    }

    pub struct Counting;

    /// Both cells are `const`-initialised, so touching them here cannot itself
    /// allocate — which would be an infinite recursion through the allocator.
    fn note() {
        let _ = WATCHING.try_with(|watching| {
            if watching.get() {
                let _ = COUNT.try_with(|count| count.set(count.get() + 1));
            }
        });
    }

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            note();
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            note();
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            note();
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            note();
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    /// Run `body` and report how many times the allocator was reached.
    ///
    /// Counts frees as well as allocations: freeing on the audio thread takes
    /// the same lock and is the more likely mistake, because it happens
    /// silently at the end of a scope.
    pub fn allocations_during(body: impl FnOnce()) -> usize {
        COUNT.with(|count| count.set(0));
        WATCHING.with(|watching| watching.set(true));
        body();
        WATCHING.with(|watching| watching.set(false));
        COUNT.with(|count| count.get())
    }
}

#[cfg(test)]
#[global_allocator]
static COUNTING_ALLOCATOR: counting_allocator::Counting = counting_allocator::Counting;

#[cfg(test)]
mod tests {
    use super::*;
    use engine::pattern::{Lane, LaneTrack, Note, Part, Scale, PPQ};
    use std::path::Path;

    fn shipped_kit() -> Arc<Kit> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data")
            .join("kits")
            .join("trap-default");
        Arc::new(Kit::load(&dir).expect("the shipped kit must load"))
    }

    /// An engine with no device behind it, driven by hand.
    fn harness(kit: Arc<Kit>, rate: f64, channels: usize) -> (Engine, rtrb::Producer<Command>) {
        let (tx, rx) = rtrb::RingBuffer::new(RING_CAPACITY);
        let (retire_tx, _retire_rx) = rtrb::RingBuffer::new(RING_CAPACITY);
        let engine = Engine {
            kit,
            sampler: Sampler::default(),
            transport: Transport::default(),
            commands: rx,
            retire: retire_tx,
            pending_retire: None,
            shared: Arc::new(Shared::new()),
            sample_rate: rate,
            channels,
        };
        (engine, tx)
    }

    fn four_on_the_floor() -> Pattern {
        Pattern {
            id: "t".into(),
            part: Part::Drums,
            artist_id: "t".into(),
            seed: 1,
            bars: 1,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            lanes: vec![LaneTrack {
                lane: Lane::Kick,
                notes: (0..4)
                    .map(|b| Note {
                        start_tick: b * PPQ,
                        len_ticks: 240,
                        pitch: 36,
                        vel: 110,
                        slide_to_pitch: None,
                        articulation: None,
                    })
                    .collect(),
            }],
            ppq: PPQ,
            mood: None,
        }
    }

    #[test]
    fn a_pattern_pushed_through_the_ring_comes_out_as_audio() {
        let kit = shipped_kit();
        let (mut engine, mut tx) = harness(Arc::clone(&kit), 48_000.0, 2);
        let schedule = transport::schedule(&four_on_the_floor(), &kit, 48_000.0);
        tx.push(Command::Play(Box::new(schedule))).unwrap();

        let mut out = vec![0.0f32; 256 * 2];
        engine.process(&mut out);
        assert!(
            out.iter().any(|s| s.abs() > 0.01),
            "the first buffer after play must already contain the downbeat"
        );
    }

    #[test]
    fn the_first_hit_lands_inside_the_first_buffer() {
        // FR-014: trigger latency under 10 ms at a 256-frame buffer. The
        // sequencer starts at sample 0, so the bound is the buffer itself —
        // this asserts nothing is deferred to the *next* callback.
        let kit = shipped_kit();
        let (mut engine, mut tx) = harness(Arc::clone(&kit), 48_000.0, 2);
        tx.push(Command::Play(Box::new(transport::schedule(
            &four_on_the_floor(),
            &kit,
            48_000.0,
        ))))
        .unwrap();

        let mut out = vec![0.0f32; 256 * 2];
        engine.process(&mut out);
        let first = out
            .chunks(2)
            .position(|f| f[0].abs() > 0.001)
            .expect("something must sound in the first buffer");
        let latency_ms = first as f64 / 48.0;
        assert!(latency_ms < 10.0, "first hit at {latency_ms} ms");
    }

    #[test]
    fn stopping_leaves_the_tail_ringing_rather_than_chopping_it() {
        // A stop that truncates a decaying 808 is a click, and it is the most
        // common way a transport sounds broken.
        let kit = shipped_kit();
        let (mut engine, mut tx) = harness(Arc::clone(&kit), 48_000.0, 2);
        tx.push(Command::Play(Box::new(transport::schedule(
            &four_on_the_floor(),
            &kit,
            48_000.0,
        ))))
        .unwrap();

        let mut out = vec![0.0f32; 64 * 2];
        engine.process(&mut out);

        tx.push(Command::Stop).unwrap();
        engine.process(&mut out);
        assert!(
            out.iter().any(|s| s.abs() > 0.0001),
            "the kick that was already sounding must ring on"
        );
        assert!(!engine.transport.playing());
    }

    #[test]
    fn a_replaced_schedule_is_handed_back_and_never_freed_on_the_audio_thread() {
        let kit = shipped_kit();
        let (tx_ring, rx_ring) = rtrb::RingBuffer::new(RING_CAPACITY);
        let (retire_tx, mut retire_rx) = rtrb::RingBuffer::new(RING_CAPACITY);
        let mut tx = tx_ring;
        let mut engine = Engine {
            kit: Arc::clone(&kit),
            sampler: Sampler::default(),
            transport: Transport::default(),
            commands: rx_ring,
            retire: retire_tx,
            pending_retire: None,
            shared: Arc::new(Shared::new()),
            sample_rate: 48_000.0,
            channels: 2,
        };

        let mut out = vec![0.0f32; 128 * 2];
        for _ in 0..2 {
            tx.push(Command::Play(Box::new(transport::schedule(
                &four_on_the_floor(),
                &kit,
                48_000.0,
            ))))
            .unwrap();
            engine.process(&mut out);
        }

        assert!(
            retire_rx.pop().is_ok(),
            "the first schedule should have come back out"
        );
    }

    #[test]
    fn the_audio_callback_never_allocates_or_frees() {
        // FR-014. The callback runs on a deadline the OS enforces; an
        // allocation can block on a lock held by any other thread, and the
        // output underruns. This drives the real engine through a full loop —
        // triggers firing, voices being stolen, the transport turning over —
        // and requires the allocator to be untouched throughout.
        let kit = shipped_kit();
        let (mut engine, mut tx) = harness(Arc::clone(&kit), 48_000.0, 2);
        tx.push(Command::Play(Box::new(transport::schedule(
            &four_on_the_floor(),
            &kit,
            48_000.0,
        ))))
        .unwrap();

        let mut out = vec![0.0f32; 256 * 2];
        // One block first: this is where the schedule is installed, and the
        // buffer above is already allocated. Everything after is steady state.
        engine.process(&mut out);

        // 400 blocks at 256 frames is ~2.1 seconds — two full turns of a
        // 120 BPM bar, so the loop boundary is crossed inside the window.
        let allocations = counting_allocator::allocations_during(|| {
            for _ in 0..400 {
                engine.process(&mut out);
            }
        });

        assert_eq!(
            allocations, 0,
            "the audio callback reached the allocator {allocations} times"
        );
    }

    #[test]
    fn the_shipped_kit_can_play_every_note_the_shipped_models_generate() {
        // A lane with no pad is silent, and silence is indistinguishable from a
        // generator that wrote nothing. This is the test that decides whether
        // the kit is complete — if a model starts generating a lane the kit
        // does not have, it fails here rather than in someone's ears.
        use engine::context::{Humanize, SessionContext, Swing, SwingGrid};

        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data");
        let scan = engine::dataset::files::scan(&dir).unwrap();
        let (models, _) = engine::dataset::registry_from(scan.files).resolve_all();
        let kit = shipped_kit();

        let context = SessionContext {
            bpm: 140.0,
            time_sig_num: 4,
            time_sig_den: 4,
            key_root: 0,
            scale: Scale::NaturalMinor,
            swing: Swing {
                grid: SwingGrid::Sixteenth,
                amount: 0.54,
            },
            bars: 4,
            half_time: false,
            humanize: Humanize::default(),
        };

        for (id, model) in &models {
            if id.starts_with('_') {
                continue;
            }
            for seed in 0..8u64 {
                let lanes = engine::generators::drums::generate(model, &context, seed);
                let pattern = Pattern {
                    id: id.clone(),
                    part: Part::Drums,
                    artist_id: id.clone(),
                    seed,
                    bars: 4,
                    bpm: 140.0,
                    time_sig_num: 4,
                    time_sig_den: 4,
                    key_root: 0,
                    scale: Scale::NaturalMinor,
                    lanes,
                    ppq: PPQ,
                    mood: None,
                };
                let schedule = transport::schedule(&pattern, &kit, 48_000.0);
                assert_eq!(
                    schedule.unplaced, 0,
                    "{id} seed {seed} generates notes the preview kit cannot play"
                );
            }
        }
    }

    #[test]
    fn a_machine_with_no_sound_card_still_gets_an_app() {
        // Playback is the only thing that may be missing. `Audio::start`
        // against a directory with no kit in it must come back carrying the
        // reason rather than panicking on the way to the first frame.
        let audio = Audio::start(Path::new("definitely/not/a/kit"));
        let failure = audio
            .failure
            .clone()
            .expect("a missing kit must be reported");
        assert!(failure.contains("kit.json"), "{failure}");
        assert!(audio.kit.pads.is_empty());

        // And a command against it is refused with that reason, not ignored.
        let refused = play_pattern_inner(&audio, &four_on_the_floor(), Some(true)).unwrap_err();
        assert!(refused.contains("kit.json"), "{refused}");
        // Stop stays quiet: stopping something that never started is not an
        // error a user needs to see.
        assert!(stop_inner(&audio).is_ok());
    }

    #[test]
    fn rebuilding_the_rings_redirects_commands_to_the_new_engine() {
        // The heart of device recovery: the old engine owns one end of each
        // ring and dies with the stream, so the UI's ends have to be replaced
        // underneath it. A command sent after a swap must reach the *new*
        // consumer — if it still went to the old one it would vanish, and the
        // transport would look alive while doing nothing.
        let (channels, mut old_commands, _old_retire) = Channels::new();

        channels
            .commands
            .lock()
            .unwrap()
            .push(Command::Stop)
            .unwrap();
        assert!(old_commands.pop().is_ok(), "the first ring should work");

        let (mut new_commands, _new_retire) = channels.rebuild();
        channels
            .commands
            .lock()
            .unwrap()
            .push(Command::Stop)
            .unwrap();

        assert!(new_commands.pop().is_ok(), "the new engine must receive it");
        assert!(old_commands.pop().is_err(), "the dead engine must not");
    }

    #[test]
    fn a_queued_command_dies_with_the_device_it_was_addressed_to() {
        // Replaying a queued `play` into a freshly opened stream would start
        // audio nobody asked for, seconds after an unplug.
        let (channels, mut old_commands, _old_retire) = Channels::new();
        channels
            .commands
            .lock()
            .unwrap()
            .push(Command::SetLooping(true))
            .unwrap();

        let (mut new_commands, _new_retire) = channels.rebuild();
        assert!(new_commands.pop().is_err(), "nothing carries over");
        // And the old ring is unreachable from the app now, whatever is in it.
        assert!(old_commands.pop().is_ok());
    }

    /// What a harnessed `tend_device` lets a test see.
    struct Harness {
        attempts: Arc<std::sync::atomic::AtomicUsize>,
        /// The UI's ends, which recovery is supposed to replace.
        channels: Arc<Channels>,
        /// The consumer handed to the most recent successful build — i.e. the
        /// one the *new* engine would be reading.
        engine_end: Arc<Mutex<Option<rtrb::Consumer<Command>>>>,
    }

    /// Drive `tend_device` on its own thread with a builder we control, then
    /// read the state it publishes. The real one needs a sound card; this
    /// needs only the state machine.
    fn tend_with(shared: &Arc<Shared>, outcomes: Vec<Result<(), String>>) -> Harness {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let engine_end: Arc<Mutex<Option<rtrb::Consumer<Command>>>> = Arc::new(Mutex::new(None));
        let kit = Arc::new(Kit {
            id: "none".into(),
            pads: Vec::new(),
        });
        let (channels, _rx, _tx) = Channels::new();
        let channels = Arc::new(channels);

        let shared_thread = Arc::clone(shared);
        let attempts_thread = Arc::clone(&attempts);
        let engine_thread = Arc::clone(&engine_end);
        let channels_thread = Arc::clone(&channels);
        std::thread::spawn(move || {
            let mut outcomes = outcomes.into_iter();
            tend_device(
                (),
                &kit,
                &channels_thread,
                &shared_thread,
                move |_kit, rx, _tx, _shared| {
                    attempts_thread.fetch_add(1, Ordering::Relaxed);
                    let outcome = outcomes.next().unwrap_or(Ok(()));
                    // Keep the consumer only when the build "succeeded", the
                    // same as a real engine taking ownership of it.
                    if outcome.is_ok() {
                        *engine_thread.lock().unwrap() = Some(rx);
                    }
                    outcome
                },
            );
        });

        Harness {
            attempts,
            channels,
            engine_end,
        }
    }

    /// Wait for a condition rather than sleeping a fixed time, so the test is
    /// not a race on a loaded machine.
    fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn a_lost_device_is_reopened_by_itself() {
        // FR-014's acceptance: unplug, and the app recovers rather than going
        // silently deaf until someone restarts it.
        let shared = Arc::new(Shared::new());
        let harness = tend_with(&shared, vec![Ok(())]);

        shared.playing.store(true, Ordering::Relaxed);
        shared.device_lost.store(true, Ordering::Relaxed);

        assert!(
            wait_for(|| harness.attempts.load(Ordering::Relaxed) >= 1),
            "a lost device must be rebuilt without being asked"
        );
        assert!(wait_for(|| shared.device_state() == DeviceState::Ok));
        assert!(
            !shared.playing.load(Ordering::Relaxed),
            "playback must not be left claiming to run on a device that went away"
        );
        assert!(
            shared.device_generation.load(Ordering::Relaxed) >= 1,
            "a successful reopen has to be visible to the publisher"
        );

        // And the recovery has to be *wired*, not merely reported. Rebuilding a
        // stream while leaving the UI pushing into the dead engine's ring is
        // the worst version of this bug: every gauge says recovered, the
        // transport looks alive, and not one command arrives anywhere.
        harness
            .channels
            .commands
            .lock()
            .unwrap()
            .push(Command::Stop)
            .expect("the app's producer must still accept commands");
        assert!(
            harness
                .engine_end
                .lock()
                .unwrap()
                .as_mut()
                .expect("a successful build takes the consumer")
                .pop()
                .is_ok(),
            "a command sent after recovery must reach the new engine"
        );
    }

    #[test]
    fn it_keeps_trying_while_the_device_is_still_unplugged() {
        // The realistic case: the device is gone for as long as it takes
        // someone to find the cable. Giving up after one attempt leaves an app
        // that is deaf until relaunched, which is the bug being fixed.
        let shared = Arc::new(Shared::new());
        let harness = tend_with(
            &shared,
            vec![
                Err("no device".into()),
                Err("still no device".into()),
                Ok(()),
            ],
        );

        shared.device_lost.store(true, Ordering::Relaxed);

        assert!(
            wait_for(|| shared.device_state() == DeviceState::Failed),
            "a failed attempt has to be reported, not swallowed"
        );
        assert!(
            wait_for(|| shared.device_state() == DeviceState::Ok),
            "and it must keep trying until the device comes back"
        );
        assert!(
            harness.attempts.load(Ordering::Relaxed) >= 3,
            "it stopped retrying"
        );
    }

    #[test]
    fn a_healthy_device_is_left_alone() {
        // The poll runs forever; it must not rebuild a stream that is fine.
        let shared = Arc::new(Shared::new());
        let harness = tend_with(&shared, vec![]);

        std::thread::sleep(DEVICE_POLL * 4);
        assert_eq!(harness.attempts.load(Ordering::Relaxed), 0);
        assert_eq!(shared.device_state(), DeviceState::Ok);
    }

    #[test]
    fn the_playhead_reports_a_fraction_and_a_beat() {
        let shared = Arc::new(Shared::new());
        shared.loop_len.store(96_000, Ordering::Relaxed);
        shared.position.store(24_000, Ordering::Relaxed);
        shared.beats.store(4.0f64.to_bits(), Ordering::Relaxed);
        shared.playing.store(true, Ordering::Relaxed);

        let audio = Audio {
            channels: Arc::new(Channels::new().0),
            shared,
            kit: Arc::new(Kit {
                id: "none".into(),
                pads: Vec::new(),
            }),
            failure: None,
        };

        let head = playhead(&audio);
        assert!(head.playing);
        assert!((head.position - 0.25).abs() < 1e-9);
        assert!((head.beat - 1.0).abs() < 1e-9, "a quarter of four beats");
    }
}
