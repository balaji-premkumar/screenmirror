use std::process::{Command, Stdio, Child};
use std::sync::Mutex;
use once_cell::sync::Lazy;
use std::io::Write;
use crate::receiver::log_event;

pub static FFPLAY_PROCESS: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));

pub fn start_native_preview(project_root: &str) {
    log_event("INFO", "PREVIEW", "ffplay", "Launching ffplay native pipeline...");
    
    // Kill existing process if any
    if let Ok(mut handle) = FFPLAY_PROCESS.lock() {
        if let Some(mut child) = handle.take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        let bundled = format!("{}/bin/ffplay", project_root);
        let cmd = if std::path::Path::new(&bundled).exists() {
            bundled
        } else {
            "ffplay".to_string()
        };

        match Command::new(&cmd)
            .args(&[
                "-f", "hevc",             // Force format to H.265/HEVC
                "-fflags", "nobuffer",    // Reduce latency
                "-flags", "low_delay",    // Reduce latency
                "-strict", "experimental",
                "-framedrop",             // Drop frames if falling behind
                "-i", "pipe:0",           // Read from standard input
                "-window_title", "Mirror High-Speed Preview",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                log_event("SUCCESS", "PREVIEW", "ffplay", "ffplay launched successfully");
                *handle = Some(child);
            }
            Err(e) => {
                log_event("ERROR", "PREVIEW", "ffplay", &format!("Failed to launch ffplay: {}", e));
            }
        }
    }
}
