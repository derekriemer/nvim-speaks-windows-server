use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};

use anyhow::{Context, Result};
use serde_json::json;

use crate::SpeechBackend;

pub struct SapiBackend {
    child: Child,
    stdin: ChildStdin,
}

impl SapiBackend {
    pub fn new() -> Result<Self> {
        let script = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech
$speaker = [System.Speech.Synthesis.SpeechSynthesizer]::new()
while (($line = [Console]::In.ReadLine()) -ne $null) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $message = $line | ConvertFrom-Json
    if ($message.interrupt) { $speaker.SpeakAsyncCancelAll() }
    $text = [string] $message.text
    if ($null -ne $message.pitch -and [double]$message.pitch -ne 1.0) {
        $pct = [int](([double]$message.pitch - 1.0) * 100)
        $sign = if ($pct -ge 0) { '+' } else { '' }
        $escaped = [System.Security.SecurityElement]::Escape($text)
        $ssml = "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'><prosody pitch='${sign}${pct}%'>${escaped}</prosody></speak>"
        [void] $speaker.SpeakSsmlAsync($ssml)
    } else {
        [void] $speaker.SpeakAsync($text)
    }
}
"#;

        let mut child = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to start PowerShell SAPI host")?;

        let stdin = child
            .stdin
            .take()
            .context("failed to open SAPI host stdin")?;

        Ok(Self { child, stdin })
    }
}

impl SpeechBackend for SapiBackend {
    fn name(&self) -> &'static str {
        "sapi"
    }

    fn supports_pitch(&self) -> bool {
        true
    }

    fn speak(&mut self, text: &str, interrupt: bool, pitch: Option<f32>) -> Result<()> {
        let message = json!({
            "text": text,
            "interrupt": interrupt,
            "pitch": pitch,
        });

        serde_json::to_writer(&mut self.stdin, &message)?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;

        Ok(())
    }
}

impl Drop for SapiBackend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
