use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleParts {
    pub epoch: i64,
    pub timezone: String,
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionOpen {
    pub protocol: u8,
    pub login_profile: PathBuf,
    pub work_profile: PathBuf,
    pub headless: bool,
    pub timezone: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishJob {
    pub protocol: u8,
    pub attempt_id: String,
    pub uuid: String,
    pub image_path: PathBuf,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub schedule: ScheduleParts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    pub tiers: Vec<String>,
    pub galleries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum WorkerCommand {
    #[serde(rename = "open")]
    Open(SessionOpen),
    #[serde(rename = "submit")]
    Submit(PublishJob),
    #[serde(rename = "close")]
    Close { protocol: u8 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum WorkerEvent {
    #[serde(rename = "ready")]
    Ready { protocol: u8 },
    #[serde(rename = "scheduled")]
    Scheduled {
        protocol: u8,
        attempt_id: String,
        uuid: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    #[serde(rename = "failed")]
    Failed {
        protocol: u8,
        attempt_id: String,
        uuid: String,
        error: String,
    },
    #[serde(rename = "unknown")]
    Unknown {
        protocol: u8,
        attempt_id: String,
        uuid: String,
        error: String,
    },
    #[serde(rename = "fatal")]
    Fatal { protocol: u8, error: String },
}

impl WorkerCommand {
    pub fn validate(&self) -> Result<(), String> {
        let protocol = match self {
            Self::Open(x) => x.protocol,
            Self::Submit(x) => x.protocol,
            Self::Close { protocol } => *protocol,
        };
        if protocol != 1 {
            return Err(format!("unsupported worker protocol {protocol}"));
        }
        if let Self::Submit(job) = self {
            if uuid::Uuid::parse_str(&job.attempt_id).is_err()
                || uuid::Uuid::parse_str(&job.uuid).is_err()
                || job.title.is_empty()
                || job.description.is_empty()
            {
                return Err("invalid publish job identifiers or metadata".into());
            }
            if job
                .image_path
                .extension()
                .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("webp"))
            {
                return Err("WebP is not accepted by DeviantArt".into());
            }
            let s = &job.schedule;
            if s.minute != 0
                || !(1..=12).contains(&s.month)
                || !(1..=31).contains(&s.day)
                || s.hour > 23
                || chrono::NaiveDate::from_ymd_opt(s.year, s.month, s.day).is_none()
            {
                return Err("invalid schedule parts".into());
            }
            let zone = s
                .timezone
                .parse::<chrono_tz::Tz>()
                .map_err(|_| "invalid schedule timezone".to_string())?;
            if zone
                .with_ymd_and_hms(s.year, s.month, s.day, s.hour, s.minute, 0)
                .single()
                .is_none_or(|dt| dt.timestamp() != s.epoch)
            {
                return Err("schedule epoch does not match wall time".into());
            }
        }
        Ok(())
    }
}

pub fn read_frame<R: std::io::BufRead>(reader: &mut R) -> Result<Option<WorkerCommand>, String> {
    const MAX_LINE: usize = 1024 * 1024;
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|e| e.to_string())?;
        if available.is_empty() {
            break;
        }
        let take = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if bytes.len() + take > MAX_LINE {
            return Err("worker protocol line exceeds 1 MiB".into());
        }
        let ended = available[take - 1] == b'\n';
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if ended {
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    let command: WorkerCommand = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    command.validate()?;
    Ok(Some(command))
}
