#![allow(dead_code)]
//! Video quality presets matching the Python VideoQuality model.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum VideoQuality {
    OD,
    BD,
    UHD,
    HD,
    SD,
    LD,
}

impl VideoQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            VideoQuality::OD => "OD",
            VideoQuality::BD => "BD",
            VideoQuality::UHD => "UHD",
            VideoQuality::HD => "HD",
            VideoQuality::SD => "SD",
            VideoQuality::LD => "LD",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            VideoQuality::OD => "原画",
            VideoQuality::BD => "蓝光",
            VideoQuality::UHD => "超清",
            VideoQuality::HD => "高清",
            VideoQuality::SD => "标清",
            VideoQuality::LD => "流畅",
        }
    }
}

impl fmt::Display for VideoQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl std::str::FromStr for VideoQuality {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "OD" => Ok(VideoQuality::OD),
            "BD" => Ok(VideoQuality::BD),
            "UHD" => Ok(VideoQuality::UHD),
            "HD" => Ok(VideoQuality::HD),
            "SD" => Ok(VideoQuality::SD),
            "LD" => Ok(VideoQuality::LD),
            _ => Err(format!("Unknown quality: {}", s)),
        }
    }
}
