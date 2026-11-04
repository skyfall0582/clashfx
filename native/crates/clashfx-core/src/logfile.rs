use crate::model::Activity;
use chrono::Utc;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::LazyLock,
};

pub(crate) struct LogTail {
    path: PathBuf,
    offset: u64,
    pending: Vec<u8>,
}

impl LogTail {
    pub(crate) fn new(path: PathBuf) -> Self {
        let offset = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        Self {
            path,
            offset,
            pending: Vec::new(),
        }
    }

    pub(crate) fn skip_to_end(&mut self) {
        self.offset = std::fs::metadata(&self.path).map_or(0, |meta| meta.len());
        self.pending.clear();
    }

    pub(crate) fn poll(&mut self) -> std::io::Result<Vec<Activity>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        if file.metadata()?.len() < self.offset {
            self.offset = 0;
            self.pending.clear();
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.take(256 * 1024).read_to_end(&mut bytes)?;
        self.offset += bytes.len() as u64;
        self.pending.extend(bytes);
        let Some(end) = self.pending.iter().rposition(|byte| *byte == b'\n') else {
            return Ok(Vec::new());
        };
        let complete: Vec<u8> = self.pending.drain(..=end).collect();
        Ok(complete
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| parse(&String::from_utf8_lossy(line)))
            .collect())
    }
}

fn parse(line: &str) -> Activity {
    static LOG: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"^time="([^"]+)"\s+level=(\w+)\s+msg=("(?:\\.|[^"\\])*"|.*)(.*)$"#)
            .expect("valid log format")
    });
    if let Some(parts) = LOG.captures(line.trim()) {
        let message =
            serde_json::from_str::<String>(&parts[3]).unwrap_or_else(|_| parts[3].to_owned());
        return Activity {
            at: chrono::DateTime::parse_from_rfc3339(&parts[1])
                .map_or_else(|_| Utc::now(), |time| time.with_timezone(&Utc)),
            level: if &parts[2] == "fatal" {
                "error".into()
            } else {
                parts[2].to_ascii_lowercase()
            },
            message: format!("{message}{}", &parts[4]),
        };
    }
    Activity {
        at: Utc::now(),
        level: "info".into(),
        message: line.trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn preserves_startup_warnings_partial_lines_and_truncation_without_duplicates() {
        let path = std::env::temp_dir().join(format!("clashfx-log-{}.log", uuid::Uuid::new_v4()));
        let mut tail = LogTail::new(path.clone());
        std::fs::write(
            &path,
            "time=\"2026-11-01T12:00:00Z\" level=warning msg=\"DNS timeout\\nretry\"\npartial",
        )
        .unwrap();
        let entries = tail.poll().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, "warning");
        assert_eq!(entries[0].message, "DNS timeout\nretry");
        assert!(tail.poll().unwrap().is_empty());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b" line\n")
            .unwrap();
        assert_eq!(tail.poll().unwrap()[0].message, "partial line");
        std::fs::write(&path, "restarted\n").unwrap();
        assert_eq!(tail.poll().unwrap()[0].message, "restarted");
        std::fs::remove_file(path).unwrap();
    }
}
