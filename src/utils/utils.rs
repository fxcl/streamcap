#![allow(dead_code)]
//! Utility functions (formatting, filename cleanup, disk checks).


use std::path::Path;

/// 检查磁盘剩余空间 (GB)
pub fn check_disk_free_space(path: &Path) -> f64 {
    let dir = if path.is_file() { path.parent().unwrap_or(path) } else { path };

    if let Ok(total_free) = get_free_space(dir) {
        return total_free as f64 / (1024.0 * 1024.0 * 1024.0);
    }

    999.0
}

#[cfg(target_os = "macos")]
fn get_free_space(path: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    use std::ffi::CString;
    use std::mem;
    let path_c = CString::new(path.to_string_lossy().as_bytes())?;
    unsafe {
        let mut stat: libc::statfs = mem::zeroed();
        if libc::statfs(path_c.as_ptr(), &mut stat) == 0 {
            return Ok(stat.f_bavail as u64 * stat.f_bsize as u64);
        }
    }
    Err("statfs failed".into())
}

#[cfg(target_os = "linux")]
fn get_free_space(path: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    use std::ffi::CString;
    use std::mem;
    let path_c = CString::new(path.to_string_lossy().as_bytes())?;
    unsafe {
        let mut stat: libc::statfs = mem::zeroed();
        if libc::statfs(path_c.as_ptr(), &mut stat) == 0 {
            return Ok(stat.f_bavail as u64 * stat.f_bsize as u64);
        }
    }
    Err("statfs failed".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn get_free_space(_path: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    Err("unsupported platform".into())
}

/// 清理文件名中的特殊字符
pub fn clean_filename(input: &str) -> String {
    let result: String = input
        .trim()
        .replace('（', "(")
        .replace('）', ")")
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '&' | '#' | '.' | '，' | ',' | '~' | '！' | '·' | ' ' => '_',
            _ => c,
        })
        .collect();

    let result = remove_emojis(&result, "_");
    let result = result.replace("__", "_");
    result.trim_matches('_').to_string()
}

fn remove_emojis(text: &str, replacement: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        if is_emoji(ch) {
            result.push_str(replacement);
        } else {
            result.push(ch);
        }
    }
    result
}

fn is_emoji(ch: char) -> bool {
    matches!(ch as u32,
        0x1F1E0..=0x1F1FF | 0x1F300..=0x1F5FF | 0x1F600..=0x1F64F |
        0x1F680..=0x1F6FF | 0x1F700..=0x1F77F | 0x1F780..=0x1F7FF |
        0x1F800..=0x1F8FF | 0x1F900..=0x1F9FF | 0x1FA00..=0x1FA6F |
        0x1FA70..=0x1FAFF | 0x2702..=0x27B0
    )
}

/// 获取 URL 查询参数
pub fn get_query_params(url: &str, param: &str) -> Option<String> {
    url::Url::parse(url).ok().and_then(|parsed| {
        parsed.query_pairs()
            .find(|(k, _)| k == param)
            .map(|(_, v)| v.to_string())
    })
}

/// 生成文件名
pub fn generate_filename(
    anchor_name: &str,
    title: Option<&str>,
    platform: Option<&str>,
    custom_template: Option<&str>,
) -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let time_str = format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );

    if let Some(template) = custom_template {
        let mut filename = template.to_string();
        filename = filename.replace("{anchor_name}", anchor_name);
        filename = filename.replace("{title}", title.unwrap_or(""));
        filename = filename.replace("{time}", &time_str);
        filename = filename.replace("{platform}", platform.unwrap_or(""));

        while filename.contains("__") {
            filename = filename.replace("__", "_");
        }
        filename = filename.trim_matches('_').to_string();

        if filename.is_empty() {
            filename = [anchor_name, title.unwrap_or(""), &time_str]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<&str>>()
                .join("_");
        }
        return filename;
    }

    [anchor_name, title.unwrap_or(""), &time_str]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("_")
}

/// 格式化时间间隔 (HH:MM:SS)
pub fn format_duration(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
