//! 会话录制 — asciicast v2(首行 header,随后 [t, "o", data] 事件行,逐条 flush)

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use crate::error::Result;

pub struct Recorder {
    writer: BufWriter<File>,
    start: Instant,
    pub path: PathBuf,
    pub started_at_ms: i64,
}

impl Recorder {
    pub fn start(dir: &std::path::Path, host_name: &str, cols: u16, rows: u16) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = dir.join(format!("{host_name}-{ts}.cast"));
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);
        let header = serde_json::json!({
            "version": 2,
            "width": cols,
            "height": rows,
            "timestamp": chrono::Utc::now().timestamp(),
            "env": { "TERM": "xterm-256color" }
        });
        writeln!(writer, "{header}")?;
        writer.flush()?;
        Ok(Self {
            writer,
            start: Instant::now(),
            path,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
        })
    }

    pub fn write_output(&mut self, data: &[u8]) {
        let t = self.start.elapsed().as_secs_f64();
        let text = String::from_utf8_lossy(data);
        let line = serde_json::json!([t, "o", text]);
        let _ = writeln!(self.writer, "{line}");
        let _ = self.writer.flush();
    }

    pub fn duration_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}
