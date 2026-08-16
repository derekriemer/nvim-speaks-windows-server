use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, ValueEnum};
use serde_json::json;

mod protocol;

#[cfg(windows)]
mod windows_backends;

use protocol::{Envelope, Speech, SpeechCommand};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BackendChoice {
    Auto,
    Nvda,
    Sapi,
    Log,
}

#[derive(Debug, Parser)]
#[command(version, about = "stdio speech server for nvim-speaks on Windows")]
struct Args {
    #[arg(long, value_enum, default_value_t = BackendChoice::Auto)]
    backend: BackendChoice,

    #[arg(long)]
    dll: Option<PathBuf>,

    #[arg(long)]
    no_interrupt: bool,

    /// Host address to bind in TCP mode (default: 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Listen on this TCP port instead of stdin/stdout (e.g. 7533)
    #[arg(long)]
    port: Option<u16>,
}

trait SpeechBackend: Send {
    fn name(&self) -> &'static str;
    fn speak(&mut self, text: &str, interrupt: bool, pitch: Option<f32>) -> Result<()>;
    fn supports_pitch(&self) -> bool {
        false
    }
}

#[cfg(windows)]
extern "system" {
    fn Beep(dw_freq: u32, dw_duration: u32) -> i32;
}

fn play_earcon(name: &str) {
    if let Some(tones) = earcon_tone(name) {
        for &(freq, duration) in tones {
            play_beep(freq, duration);
        }
    }
}

fn earcon_tone(name: &str) -> Option<&'static [(u32, u32)]> {
    match name {
        "cap" => Some(&[(880, 60)]),
        "cursor" => Some(&[(520, 25)]),
        "boundary" => Some(&[(220, 100)]),
        "completion_open" => Some(&[(600, 40)]),
        "completion_accept" => Some(&[(700, 60)]),
        "completion_dismiss" => Some(&[(300, 40)]),
        "fold_closed" => Some(&[(392, 35), (415, 35), (440, 45)]),
        "fold_open" => Some(&[(392, 35), (523, 40), (659, 55)]),
        "fold_close" => Some(&[(659, 35), (523, 40), (392, 55)]),
        "fold_none" => Some(&[(180, 45)]),
        _ => None,
    }
}

fn play_beep(freq: u32, duration: u32) {
    let _ = (freq, duration);
    #[cfg(windows)]
    {
        unsafe { Beep(freq, duration) };
    }
}

fn supported_commands(backend: &dyn SpeechBackend) -> Vec<&'static str> {
    let mut commands = vec!["text"];
    if cfg!(windows) {
        commands.push("beep");
        commands.push("earcon");
    }
    if backend.supports_pitch() {
        commands.push("pitch");
    }
    commands
}

fn supported_earcons() -> Vec<&'static str> {
    vec![
        "cap",
        "cursor",
        "boundary",
        "completion_open",
        "completion_accept",
        "completion_dismiss",
        "fold_closed",
        "fold_open",
        "fold_close",
        "fold_none",
    ]
}

fn capabilities_json(backend: &dyn SpeechBackend) -> serde_json::Value {
    json!({
        "type": "capabilities",
        "backend": backend.name(),
        "commands": supported_commands(backend),
        "earcons": supported_earcons(),
    })
}

struct SpeechTask {
    speech: Speech,
    seq: u64,
    reply_tx: mpsc::Sender<SpeechReply>,
}

struct SpeechReply {
    seq: u64,
    backend_name: &'static str,
    error: Option<String>,
}

struct LogBackend;

impl SpeechBackend for LogBackend {
    fn name(&self) -> &'static str {
        "log"
    }

    fn speak(&mut self, text: &str, interrupt: bool, pitch: Option<f32>) -> Result<()> {
        eprintln!(
            "speak interrupt={} pitch={:?} text={}",
            interrupt,
            pitch,
            serde_json::to_string(text).unwrap_or_else(|_| "\"<invalid>\"".to_string())
        );
        Ok(())
    }
}

fn write_msg(writer: &mut impl Write, value: &serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn positive_u32(value: Option<f64>) -> Option<u32> {
    let value = value?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    Some(value.round() as u32)
}

fn execute_command(
    backend: &mut dyn SpeechBackend,
    command: &SpeechCommand,
    interrupt: bool,
    current_pitch: Option<f32>,
) -> Result<bool> {
    match command.cmd.as_str() {
        "text" => {
            if let Some(text) = command.s.as_deref() {
                if !text.is_empty() {
                    backend.speak(text, interrupt, current_pitch)?;
                    return Ok(true);
                }
            }
        }
        "beep" => {
            if let (Some(freq), Some(duration)) =
                (positive_u32(command.hz), positive_u32(command.ms))
            {
                play_beep(freq, duration);
            }
        }
        "earcon" => {
            if let Some(id) = command.id.as_deref() {
                play_earcon(id);
            }
        }
        _ => {}
    }
    Ok(false)
}

fn speak_envelope(
    backend: &mut dyn SpeechBackend,
    speech: &Speech,
    no_interrupt: bool,
) -> Result<()> {
    let interrupt = speech.interrupt && !no_interrupt;

    if speech.sequence.is_empty() {
        if let Some(ref earcon) = speech.earcon {
            play_earcon(earcon);
        }
        if !speech.text.is_empty() {
            backend.speak(&speech.text, interrupt, speech.pitch)?;
        }
        return Ok(());
    }

    let mut current_pitch = None;
    let mut spoke_text = false;
    for command in &speech.sequence {
        if command.cmd == "pitch" {
            current_pitch = command.multiplier;
            continue;
        }

        if execute_command(backend, command, interrupt && !spoke_text, current_pitch)? {
            spoke_text = true;
        }
    }

    Ok(())
}

fn run_loop(
    reader: impl BufRead,
    mut writer: impl Write,
    backend: &mut dyn SpeechBackend,
    no_interrupt: bool,
) -> Result<()> {
    for line in reader.lines() {
        let line = line.context("failed to read")?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Envelope>(&line) {
            Ok(envelope) => {
                if let Err(err) = speak_envelope(backend, &envelope.speech, no_interrupt) {
                    write_msg(
                        &mut writer,
                        &json!({
                            "type": "error",
                            "seq": envelope.seq,
                            "backend": backend.name(),
                            "message": err.to_string()
                        }),
                    )?;
                    eprintln!("speech error: {err:#}");
                }
            }
            Err(err) => {
                write_msg(
                    &mut writer,
                    &json!({
                        "type": "error",
                        "message": format!("invalid protocol line: {err}")
                    }),
                )?;
                eprintln!("invalid protocol line: {err}");
            }
        }
    }

    Ok(())
}

fn run_speech_thread(
    mut backend: Box<dyn SpeechBackend>,
    rx: mpsc::Receiver<SpeechTask>,
    no_interrupt: bool,
) {
    for task in rx {
        let result = speak_envelope(&mut *backend, &task.speech, no_interrupt);
        let _ = task.reply_tx.send(SpeechReply {
            seq: task.seq,
            backend_name: backend.name(),
            error: result.err().map(|e| format!("{e:#}")),
        });
    }
}

fn handle_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    speech_tx: mpsc::Sender<SpeechTask>,
    backend_name: &'static str,
    commands: Vec<&'static str>,
    earcons: Vec<&'static str>,
) {
    let reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(err) => {
            eprintln!("failed to clone TCP stream for {peer}: {err:#}");
            return;
        }
    };
    let mut writer = stream;

    if let Err(err) = write_msg(
        &mut writer,
        &json!({ "type": "ready", "backend": backend_name }),
    ) {
        eprintln!("failed to send ready to {peer}: {err:#}");
        return;
    }
    {
        let _ = write_msg(
            &mut writer,
            &json!({
                "type": "capabilities",
                "backend": backend_name,
                "commands": commands,
                "earcons": earcons,
            }),
        );
    }

    let (reply_tx, reply_rx) = mpsc::channel::<SpeechReply>();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(err) => {
                eprintln!("client {peer} read error: {err:#}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Envelope>(&line) {
            Ok(envelope) => {
                let task = SpeechTask {
                    speech: envelope.speech,
                    seq: envelope.seq,
                    reply_tx: reply_tx.clone(),
                };
                if speech_tx.send(task).is_err() {
                    eprintln!("speech thread gone, dropping client {peer}");
                    break;
                }
                match reply_rx.recv() {
                    Ok(reply) => {
                        if let Some(err_msg) = reply.error {
                            let _ = write_msg(
                                &mut writer,
                                &json!({
                                    "type": "error",
                                    "seq": reply.seq,
                                    "backend": reply.backend_name,
                                    "message": err_msg
                                }),
                            );
                            eprintln!("speech error for {peer}: {err_msg}");
                        }
                    }
                    Err(_) => {
                        eprintln!("speech thread gone waiting for reply, dropping client {peer}");
                        break;
                    }
                }
            }
            Err(err) => {
                let _ = write_msg(
                    &mut writer,
                    &json!({
                        "type": "error",
                        "message": format!("invalid protocol line: {err}")
                    }),
                );
                eprintln!("invalid protocol line from {peer}: {err}");
            }
        }
    }

    eprintln!("client {peer} disconnected");
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut backend = select_backend(args.backend, args.dll.as_ref())?;

    if let Some(port) = args.port {
        let addr = format!("{}:{port}", args.host);
        let listener =
            TcpListener::bind(&addr).with_context(|| format!("failed to bind {addr}"))?;
        eprintln!("listening on {addr}");

        let backend_name = backend.name();
        let commands = supported_commands(&*backend);
        let earcons = supported_earcons();
        let (speech_tx, speech_rx) = mpsc::channel::<SpeechTask>();
        let no_interrupt = args.no_interrupt;
        thread::spawn(move || run_speech_thread(backend, speech_rx, no_interrupt));

        loop {
            let (stream, peer) = listener.accept().context("accept failed")?;
            eprintln!("client connected: {peer}");
            let tx = speech_tx.clone();
            let conn_commands = commands.clone();
            let conn_earcons = earcons.clone();
            thread::spawn(move || {
                handle_connection(stream, peer, tx, backend_name, conn_commands, conn_earcons)
            });
        }
    } else {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let stdin_lock = stdin.lock();
        let mut stdout_lock = stdout.lock();
        write_msg(
            &mut stdout_lock,
            &json!({ "type": "ready", "backend": backend.name() }),
        )?;
        write_msg(&mut stdout_lock, &capabilities_json(&*backend))?;
        run_loop(stdin_lock, stdout_lock, &mut *backend, args.no_interrupt)?;
        Ok(())
    }
}

#[cfg(windows)]
fn select_backend(choice: BackendChoice, dll: Option<&PathBuf>) -> Result<Box<dyn SpeechBackend>> {
    match choice {
        BackendChoice::Auto => {
            match windows_backends::nvda::NvdaBackend::new(dll.cloned()) {
                Ok(backend) => return Ok(Box::new(backend)),
                Err(err) => eprintln!("NVDA backend unavailable: {err:#}"),
            }

            match windows_backends::sapi::SapiBackend::new() {
                Ok(backend) => return Ok(Box::new(backend)),
                Err(err) => eprintln!("SAPI backend unavailable: {err:#}"),
            }

            Ok(Box::new(LogBackend))
        }
        BackendChoice::Nvda => Ok(Box::new(windows_backends::nvda::NvdaBackend::new(
            dll.cloned(),
        )?)),
        BackendChoice::Sapi => Ok(Box::new(windows_backends::sapi::SapiBackend::new()?)),
        BackendChoice::Log => Ok(Box::new(LogBackend)),
    }
}

#[cfg(not(windows))]
fn select_backend(choice: BackendChoice, _dll: Option<&PathBuf>) -> Result<Box<dyn SpeechBackend>> {
    match choice {
        BackendChoice::Auto | BackendChoice::Log => Ok(Box::new(LogBackend)),
        BackendChoice::Nvda | BackendChoice::Sapi => Err(anyhow!(
            "{choice:?} backend is only available on Windows; use --backend log on this platform"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingBackend {
        calls: Vec<(String, bool, Option<f32>)>,
    }

    impl SpeechBackend for RecordingBackend {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn supports_pitch(&self) -> bool {
            true
        }

        fn speak(&mut self, text: &str, interrupt: bool, pitch: Option<f32>) -> Result<()> {
            self.calls.push((text.to_string(), interrupt, pitch));
            Ok(())
        }
    }

    #[test]
    fn non_windows_auto_selects_log_backend() {
        let backend = select_backend(BackendChoice::Auto, None).unwrap();
        assert_eq!(backend.name(), "log");
    }

    #[test]
    fn log_backend_speaks() {
        let mut backend = LogBackend;
        backend.speak("hello", true, None).unwrap();
        backend.speak("cap a", true, Some(1.5)).unwrap();
    }

    #[test]
    fn sequence_speaks_text_with_current_pitch() {
        let envelope: Envelope = serde_json::from_str(
            r#"{
              "seq": 1,
              "speech": {
                "interrupt": true,
                "sequence": [
                  {"cmd": "pitch", "multiplier": 1.5},
                  {"cmd": "text", "s": "cap a"},
                  {"cmd": "pitch", "multiplier": 1.0},
                  {"cmd": "text", "s": "next"}
                ]
              }
            }"#,
        )
        .unwrap();
        let mut backend = RecordingBackend { calls: Vec::new() };

        speak_envelope(&mut backend, &envelope.speech, false).unwrap();

        assert_eq!(
            backend.calls,
            vec![
                ("cap a".to_string(), true, Some(1.5)),
                ("next".to_string(), false, Some(1.0)),
            ]
        );
    }

    #[test]
    fn capabilities_advertise_sequence_commands() {
        let backend = RecordingBackend { calls: Vec::new() };
        let payload = capabilities_json(&backend);

        assert_eq!(payload["type"], "capabilities");
        assert_eq!(payload["commands"][0], "text");
        assert!(payload["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cmd| cmd == "pitch"));
        assert!(payload["earcons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "fold_open"));
    }
}
