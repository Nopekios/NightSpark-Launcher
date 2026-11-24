include!(concat!(env!("OUT_DIR"), "/watermark.rs"));

use serde::Serialize;

#[derive(Serialize)]
pub struct Watermark {
    pub tag: &'static str,
    pub serial: &'static str,
    pub commit: &'static str,
    pub built_at: &'static str,
    pub version: &'static str,
    pub official: bool,
}

pub fn info() -> Watermark {
    Watermark {
        tag: TAG,
        serial: SERIAL,
        commit: COMMIT,
        built_at: BUILT_AT,
        version: VERSION,
        official: OFFICIAL,
    }
}
