include!(concat!(env!("OUT_DIR"), "/watermark.rs"));

use serde::Serialize;
use std::io::Write;

#[derive(Serialize)]
pub struct Watermark {
    pub tag: &'static str,
    pub serial: &'static str,
    pub commit: &'static str,
    pub built_at: &'static str,
    pub version: &'static str,
    pub official: bool,
    pub integrity: bool,
}

fn fnv(parts: &[&str]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for part in parts {
        for b in part.as_bytes() {
            h ^= *b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    h
}

pub fn integrity() -> bool {
    let joined = format!("{SEG0}{SEG1}{SEG2}{SEG3}");
    if joined != TAG {
        return false;
    }
    if !TAG.starts_with("NS-") || !TAG.contains(SERIAL) || !TAG.contains(COMMIT) {
        return false;
    }
    if SEG0.is_empty() || SEG1.is_empty() || SEG2.is_empty() || SEG3.is_empty() {
        return false;
    }
    if SERIAL.len() != 12 || BUILT_AT.len() < 7 || VERSION.is_empty() {
        return false;
    }
    if FP0.len() != 16 || FP1.len() != 16 || FP2.len() != 16 || FP3.len() != 16 {
        return false;
    }
    if FP0 == FP1 || FP1 == FP2 || FP2 == FP3 || FP0 == FP2 {
        return false;
    }
    if STAMP < 1_600_000_000 || STAMP > 4_200_000_000 {
        return false;
    }
    let flag = if OFFICIAL { "1" } else { "0" };
    fnv(&[TAG, SERIAL, COMMIT, BUILT_AT, VERSION, flag]) == CHK
}

pub fn info() -> Watermark {
    Watermark {
        tag: TAG,
        serial: SERIAL,
        commit: COMMIT,
        built_at: BUILT_AT,
        version: VERSION,
        official: OFFICIAL,
        integrity: integrity(),
    }
}

pub fn write_boot_stamp() {
    let base = match std::env::var("APPDATA") {
        Ok(v) => v,
        Err(_) => return,
    };
    let dir = std::path::Path::new(&base).join("NightSpark").join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let line = format!(
        "{} tag={TAG} official={OFFICIAL} integrity={} ver={VERSION} built_at={BUILT_AT}\n",
        unix_now(),
        integrity()
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("watermark.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
