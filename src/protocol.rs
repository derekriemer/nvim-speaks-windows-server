use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Envelope {
    pub seq: u64,
    pub speech: Speech,
}

#[derive(Debug, Deserialize)]
pub struct Speech {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub interrupt: bool,
    #[serde(default)]
    pub sequence: Vec<SpeechCommand>,
    #[serde(default)]
    pub pitch: Option<f32>,
    #[serde(default)]
    pub earcon: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SpeechCommand {
    pub cmd: String,
    #[serde(default)]
    pub s: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub hz: Option<f64>,
    #[serde(default)]
    pub ms: Option<f64>,
    #[serde(default)]
    pub multiplier: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_envelope_and_ignores_unknown_fields() {
        let envelope: Envelope = serde_json::from_str(
            r#"{
              "type": "cursor_move",
              "seq": 42,
              "time": 123,
              "event": { "type": "cursor_move" },
              "speech": {
                "text": "alpha",
                "priority": "normal",
                "interrupt": true,
                "category": "navigation"
              }
            }"#,
        )
        .unwrap();

        assert_eq!(envelope.seq, 42);
        assert_eq!(envelope.speech.text, "alpha");
        assert!(envelope.speech.interrupt);
        assert!(envelope.speech.sequence.is_empty());
    }

    #[test]
    fn missing_speech_text_defaults_to_empty() {
        let envelope: Envelope = serde_json::from_str(
            r#"{
              "seq": 7,
              "speech": { "interrupt": false }
            }"#,
        )
        .unwrap();

        assert_eq!(envelope.speech.text, "");
        assert!(!envelope.speech.interrupt);
        assert!(envelope.speech.sequence.is_empty());
        assert!(envelope.speech.pitch.is_none());
        assert!(envelope.speech.earcon.is_none());
    }

    #[test]
    fn parses_sequence_commands() {
        let envelope: Envelope = serde_json::from_str(
            r#"{
              "seq": 1,
              "speech": {
                "text": "cap a",
                "interrupt": true,
                "sequence": [
                  {"cmd": "earcon", "id": "cap"},
                  {"cmd": "pitch", "multiplier": 1.5},
                  {"cmd": "text", "s": "cap a"},
                  {"cmd": "pitch", "multiplier": 1.0}
                ]
              }
            }"#,
        )
        .unwrap();

        assert_eq!(envelope.speech.sequence.len(), 4);
        assert_eq!(envelope.speech.sequence[0].cmd, "earcon");
        assert_eq!(envelope.speech.sequence[0].id.as_deref(), Some("cap"));
        assert_eq!(envelope.speech.sequence[1].multiplier, Some(1.5));
        assert_eq!(envelope.speech.sequence[2].s.as_deref(), Some("cap a"));
    }

    #[test]
    fn still_parses_legacy_pitch_and_earcon() {
        let envelope: Envelope = serde_json::from_str(
            r#"{
              "seq": 1,
              "speech": {
                "text": "cap a",
                "interrupt": true,
                "pitch": 1.5,
                "earcon": "cap"
              }
            }"#,
        )
        .unwrap();

        assert!((envelope.speech.pitch.unwrap() - 1.5).abs() < 0.001);
        assert_eq!(envelope.speech.earcon.as_deref(), Some("cap"));
    }
}
