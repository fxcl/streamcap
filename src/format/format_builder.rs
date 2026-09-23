#![allow(dead_code)]
//! Output format and muxer configuration.


use std::ffi::CString;

use rsmpeg::avcodec::AVCodecParameters;
use rsmpeg::avformat::{AVFormatContextOutput, AVOutputFormat};
use rsmpeg::avutil::{AVDictionary, AVRational};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Mp4,
    Ts,
    Mkv,
    Flv,
    Mov,
    Nut,
    Aac,
    M4a,
    Mp3,
    Wav,
    Wma,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mp4" => Ok(OutputFormat::Mp4),
            "ts" => Ok(OutputFormat::Ts),
            "mkv" => Ok(OutputFormat::Mkv),
            "flv" => Ok(OutputFormat::Flv),
            "mov" => Ok(OutputFormat::Mov),
            "nut" => Ok(OutputFormat::Nut),
            "aac" => Ok(OutputFormat::Aac),
            "m4a" => Ok(OutputFormat::M4a),
            "mp3" => Ok(OutputFormat::Mp3),
            "wav" => Ok(OutputFormat::Wav),
            "wma" => Ok(OutputFormat::Wma),
            _ => Err(format!("Unsupported format: {}", s)),
        }
    }
}

impl OutputFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputFormat::Mp4 => "mp4",
            OutputFormat::Ts => "ts",
            OutputFormat::Mkv => "mkv",
            OutputFormat::Flv => "flv",
            OutputFormat::Mov => "mov",
            OutputFormat::Nut => "nut",
            OutputFormat::Aac => "aac",
            OutputFormat::M4a => "m4a",
            OutputFormat::Mp3 => "mp3",
            OutputFormat::Wav => "wav",
            OutputFormat::Wma => "wma",
        }
    }

    pub fn file_extension(&self) -> &'static str {
        self.as_str()
    }
}

#[derive(Debug, Clone, Default)]
pub struct FormatConfig {
    pub format: OutputFormat,
    pub segment_record: bool,
    pub segment_time: u32,
    pub video_bitrate: Option<u32>,
    pub movflags: Option<String>,
    pub mpegts_flags: Option<String>,
}

impl FormatConfig {
    /// 构建分段录制的 write_header options 字典
    pub fn segment_options_dict(&self) -> Option<AVDictionary> {
        if !self.segment_record {
            return None;
        }

        let seg_time = if self.segment_time > 0 {
            self.segment_time
        } else {
            1800
        };
        let seg_time_str = seg_time.to_string();

        let (k0, v0) = (c"format", c"segment");
        let dict = AVDictionary::new(k0, v0, 0);
        let dict = dict.set(c"segment_time", &CString::new(seg_time_str.as_str()).unwrap(), 0);
        let dict = dict.set(c"segment_start_number", c"0", 0);
        let dict = dict.set(c"reset_timestamps", c"1", 0);
        let dict = dict.set(c"strftime", c"0", 0);
        let dict = dict.set(
            c"segment_format",
            &CString::new(self.format.as_str()).unwrap(),
            0,
        );

        Some(dict)
    }

    /// 构建 write_header 用的 AVDictionary (包括 movflags)
    pub fn write_header_dict(&self) -> Option<AVDictionary> {
        let mut pairs: Vec<(String, String)> = Vec::new();

        // 分段录制选项
        if self.segment_record {
            let seg_time = if self.segment_time > 0 {
                self.segment_time
            } else {
                1800
            };
            pairs.push(("format".to_string(), "segment".to_string()));
            pairs.push(("segment_time".to_string(), seg_time.to_string()));
            pairs.push(("segment_format".to_string(), self.format.as_str().to_string()));
            pairs.push(("reset_timestamps".to_string(), "1".to_string()));
            pairs.push(("strftime".to_string(), "0".to_string()));
        }

        // mp4 fragmented 模式: 直播录制中断也能播放
        if self.format == OutputFormat::Mp4 {
            // 有用户自定义 movflags 就用用户的; 否则默认 fragmented
            let flags = self
                .movflags
                .clone()
                .unwrap_or_else(|| "+frag_keyframe+empty_moov+default_base_moof".to_string());
            pairs.push(("movflags".to_string(), flags));
        }

        if pairs.is_empty() {
            return None;
        }

        // 构建 AVDictionary
        let (first_key, first_val) = &pairs[0];
        let k = CString::new(first_key.as_str()).unwrap();
        let v = CString::new(first_val.as_str()).unwrap();
        let dict = AVDictionary::new(&k, &v, 0);
        Some(pairs[1..].iter().fold(dict, |d, (key, val)| {
            d.set(
                &CString::new(key.as_str()).unwrap(),
                &CString::new(val.as_str()).unwrap(),
                0,
            )
        }))
    }
}

pub struct FormatBuilder {
    config: FormatConfig,
}

impl FormatBuilder {
    pub fn new(config: FormatConfig) -> Self {
        Self { config }
    }

    pub fn build_output_context(
        &self,
        filename: &CString,
        input_streams: &[(AVCodecParameters, AVRational)],
    ) -> Result<AVFormatContextOutput, super::super::recording::RecordingError> {
        let short_name = if self.config.segment_record {
            CString::new("segment").map_err(|e| {
                super::super::recording::RecordingError::FormatError(format!(
                    "Invalid segment format name: {}", e
                ))
            })?
        } else {
            CString::new(self.config.format.as_str()).map_err(|e| {
                super::super::recording::RecordingError::FormatError(format!(
                    "Invalid format name: {}", e
                ))
            })?
        };

        let oformat = AVOutputFormat::guess_format(Some(&short_name), Some(filename), None)
            .ok_or_else(|| super::super::recording::RecordingError::FormatError(format!(
                "Cannot guess output format for {:?}", self.config.format
            )))?;

        let mut output_ctx = AVFormatContextOutput::builder()
            .oformat(&oformat)
            .filename(filename)
            .build()
            .map_err(|e| super::super::recording::RecordingError::FormatError(format!(
                "Failed to create output context: {}", e
            )))?;

        for (codec_params, time_base) in input_streams {
            let mut output_stream = output_ctx.new_stream();
            output_stream.set_codecpar(codec_params.clone());
            output_stream.set_time_base(*time_base);
        }

        Ok(output_ctx)
    }

    pub fn config(&self) -> &FormatConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_parse() {
        assert_eq!("mp4".parse::<OutputFormat>().unwrap(), OutputFormat::Mp4);
        assert_eq!("ts".parse::<OutputFormat>().unwrap(), OutputFormat::Ts);
        assert_eq!("mkv".parse::<OutputFormat>().unwrap(), OutputFormat::Mkv);
        assert_eq!("flv".parse::<OutputFormat>().unwrap(), OutputFormat::Flv);
        assert!("xyz".parse::<OutputFormat>().is_err());
    }

    #[test]
    fn test_output_format_extension() {
        assert_eq!(OutputFormat::Mp4.file_extension(), "mp4");
        assert_eq!(OutputFormat::Ts.file_extension(), "ts");
    }

    #[test]
    fn test_segment_options_dict() {
        let config = FormatConfig {
            segment_record: true,
            segment_time: 3600,
            format: OutputFormat::Mp4,
            ..Default::default()
        };
        let opts = config.segment_options_dict();
        assert!(opts.is_some());
        let s = format!("{:?}", opts.unwrap());
        assert!(s.contains("segment"));
    }

    #[test]
    fn test_no_segment_options_when_disabled() {
        let config = FormatConfig {
            segment_record: false,
            ..Default::default()
        };
        assert!(config.segment_options_dict().is_none());
    }
}
