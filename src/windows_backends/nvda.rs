use std::ffi::c_int;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use libloading::{Library, Symbol};

use crate::SpeechBackend;

type NvdaSpeakText = unsafe extern "system" fn(*const u16) -> c_int;
type NvdaCancelSpeech = unsafe extern "system" fn() -> c_int;
type NvdaTestIfRunning = unsafe extern "system" fn() -> c_int;

pub struct NvdaBackend {
    _library: Library,
    speak_text: NvdaSpeakText,
    cancel_speech: NvdaCancelSpeech,
    test_if_running: NvdaTestIfRunning,
}

impl NvdaBackend {
    pub fn new(dll_path: Option<PathBuf>) -> Result<Self> {
        let path = dll_path.unwrap_or_else(|| PathBuf::from("nvdaControllerClient.dll"));
        let library = unsafe { Library::new(&path) }
            .with_context(|| format!("failed to load {}", path.display()))?;

        let speak_text = load_symbol::<NvdaSpeakText>(&library, b"nvdaController_speakText\0")?;
        let cancel_speech =
            load_symbol::<NvdaCancelSpeech>(&library, b"nvdaController_cancelSpeech\0")?;
        let test_if_running =
            load_symbol::<NvdaTestIfRunning>(&library, b"nvdaController_testIfRunning\0")?;

        let backend = Self {
            _library: library,
            speak_text,
            cancel_speech,
            test_if_running,
        };

        let status = unsafe { (backend.test_if_running)() };
        if status != 0 {
            return Err(anyhow!(
                "NVDA controller reports NVDA is not running: {status}"
            ));
        }

        Ok(backend)
    }
}

impl SpeechBackend for NvdaBackend {
    fn name(&self) -> &'static str {
        "nvda"
    }

    fn speak(&mut self, text: &str, interrupt: bool, _pitch: Option<f32>) -> Result<()> {
        if interrupt {
            let status = unsafe { (self.cancel_speech)() };
            if status != 0 {
                eprintln!("NVDA cancelSpeech returned {status}");
            }
        }

        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);

        let status = unsafe { (self.speak_text)(wide.as_ptr()) };
        if status != 0 {
            return Err(anyhow!("NVDA speakText returned {status}"));
        }

        Ok(())
    }
}

fn load_symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T> {
    let symbol: Symbol<T> = unsafe { library.get(name) }
        .with_context(|| format!("missing symbol {}", String::from_utf8_lossy(name)))?;
    Ok(*symbol)
}
