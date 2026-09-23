#![allow(dead_code)]
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingStatus {
    StoppedMonitoring,
    Monitoring,
    Recording,
    NotRecording,
    StatusChecking,
    NotInScheduledCheck,
    PreparingRecording,
    RecordingError,
    NotRecordingSpace,
    LiveStatusCheckError,
    LiveBroadcasting,
}

impl RecordingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordingStatus::StoppedMonitoring => "STOPPED_MONITORING",
            RecordingStatus::Monitoring => "MONITORING",
            RecordingStatus::Recording => "RECORDING",
            RecordingStatus::NotRecording => "NOT_RECORDING",
            RecordingStatus::StatusChecking => "STATUS_CHECKING",
            RecordingStatus::NotInScheduledCheck => "NOT_IN_SCHEDULED_CHECK",
            RecordingStatus::PreparingRecording => "PREPARING_RECORDING",
            RecordingStatus::RecordingError => "RECORDING_ERROR",
            RecordingStatus::NotRecordingSpace => "NOT_RECORDING_SPACE",
            RecordingStatus::LiveStatusCheckError => "LIVE_STATUS_CHECK_ERROR",
            RecordingStatus::LiveBroadcasting => "LIVE_BROADCASTING",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            RecordingStatus::StoppedMonitoring => "已停止监控",
            RecordingStatus::Monitoring => "监控中",
            RecordingStatus::Recording => "正在录制",
            RecordingStatus::NotRecording => "未录制",
            RecordingStatus::StatusChecking => "正在检查直播状态",
            RecordingStatus::NotInScheduledCheck => "不在定时检查范围",
            RecordingStatus::PreparingRecording => "准备录制中",
            RecordingStatus::RecordingError => "录制出错",
            RecordingStatus::NotRecordingSpace => "磁盘空间不足",
            RecordingStatus::LiveStatusCheckError => "直播状态检查失败",
            RecordingStatus::LiveBroadcasting => "直播中",
        }
    }
}

impl fmt::Display for RecordingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl std::str::FromStr for RecordingStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "STOPPED_MONITORING" => Ok(RecordingStatus::StoppedMonitoring),
            "MONITORING" => Ok(RecordingStatus::Monitoring),
            "RECORDING" => Ok(RecordingStatus::Recording),
            "NOT_RECORDING" => Ok(RecordingStatus::NotRecording),
            "STATUS_CHECKING" => Ok(RecordingStatus::StatusChecking),
            "NOT_IN_SCHEDULED_CHECK" => Ok(RecordingStatus::NotInScheduledCheck),
            "PREPARING_RECORDING" => Ok(RecordingStatus::PreparingRecording),
            "RECORDING_ERROR" => Ok(RecordingStatus::RecordingError),
            "NOT_RECORDING_SPACE" => Ok(RecordingStatus::NotRecordingSpace),
            "LIVE_STATUS_CHECK_ERROR" => Ok(RecordingStatus::LiveStatusCheckError),
            "LIVE_BROADCASTING" => Ok(RecordingStatus::LiveBroadcasting),
            _ => Err(format!("Unknown status: {}", s)),
        }
    }
}
