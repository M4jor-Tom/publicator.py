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
            job.validate()?;
        }
        Ok(())
    }
}

impl PublishJob {
    fn validate(&self) -> Result<(), String> {
        if uuid::Uuid::parse_str(&self.attempt_id).is_err()
            || uuid::Uuid::parse_str(&self.uuid).is_err()
            || self.title.is_empty()
            || self.description.is_empty()
        {
            return Err("invalid publish job identifiers or metadata".into());
        }
        if self
            .image_path
            .extension()
            .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("webp"))
        {
            return Err("WebP is not accepted by DeviantArt".into());
        }

        self.schedule.validate()
    }
}

impl ScheduleParts {
    fn wall_time(&self) -> Result<chrono::NaiveDateTime, String> {
        if self.minute != 0 {
            return Err("invalid schedule parts".into());
        }
        chrono::NaiveDate::from_ymd_opt(self.year, self.month, self.day)
            .and_then(|date| date.and_hms_opt(self.hour, self.minute, 0))
            .ok_or_else(|| "invalid schedule parts".to_string())
    }

    fn validate(&self) -> Result<(), String> {
        let wall_time = self.wall_time()?;
        let zone = self
            .timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| "invalid schedule timezone".to_string())?;
        let expected = match zone.from_local_datetime(&wall_time) {
            chrono::LocalResult::Single(dt) => Some(dt.timestamp()),
            chrono::LocalResult::Ambiguous(first, second) => {
                Some(first.timestamp().min(second.timestamp()))
            }
            chrono::LocalResult::None => None,
        };
        if expected != Some(self.epoch) {
            return Err("schedule epoch does not match wall time".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_validation_rejects_invalid_parts_before_the_timezone() {
        for (month, day, hour, minute) in [
            (0, 1, 0, 0),
            (13, 1, 0, 0),
            (1, 0, 0, 0),
            (1, 32, 0, 0),
            (2, 29, 0, 0),
            (4, 31, 0, 0),
            (1, 1, 24, 0),
            (1, 1, 0, 1),
        ] {
            let schedule = ScheduleParts {
                epoch: 0,
                timezone: "invalid".into(),
                year: 2023,
                month,
                day,
                hour,
                minute,
            };
            assert_eq!(schedule.validate().unwrap_err(), "invalid schedule parts");
        }
    }
}
