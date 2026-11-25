//! 账号头像：把 Minecraft 皮肤里那张 8x8 的「脸」抠出来，放大成 64x64 的 PNG，
//! 以 data URL 交给前端当头像用。
//!
//! - 正版账号：用 uuid 问 Mojang 要皮肤贴图地址，下载后裁头部（含 overlay 帽子层）；
//! - 离线账号 / 拉取失败：本地按用户名确定性生成一张像素脸，同一名字每次结果一致。
//!
//! 裁好的结果落在 `%APPDATA%\NightSpark\avatars\<uuid>.png` 做磁盘缓存，
//! 网络挂了也不影响界面出图。

use std::path::PathBuf;

use base64::Engine;
use serde::Deserialize;

use crate::auth::AccountKind;
use crate::error::AppError;

const PROFILE_URL: &str = "https://sessionserver.mojang.com/session/minecraft/profile/";

/// 皮肤贴图里头部的位置：头 8x8 在 (8, 8)，帽子层 overlay 在 (40, 8)，都只对 64x64 皮肤有效。
const HEAD_X: u32 = 8;
const HEAD_Y: u32 = 8;
const HAT_X: u32 = 40;

fn avatar_dir() -> Result<PathBuf, AppError> {
    let dir = dirs::config_dir()
        .ok_or_else(|| AppError::Internal("无法获取系统 config 目录".to_string()))?
        .join("NightSpark")
        .join("avatars");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn cache_path(uuid: &str) -> Result<PathBuf, AppError> {
    Ok(avatar_dir()?.join(format!("{}.png", uuid.to_lowercase())))
}

/// 拿头像的 data URL。任何一步失败都退回本地生成的像素脸，绝不把错误抛给界面。
pub async fn avatar_data_url(uuid: &str, name: &str, kind: AccountKind) -> String {
    if let Ok(path) = cache_path(uuid) {
        if let Ok(bytes) = std::fs::read(&path) {
            return to_data_url(&bytes);
        }
    }

    let png = match kind {
        AccountKind::Offline => None,
        AccountKind::Msa => fetch_official_head(uuid).await,
    };
    let png = png.unwrap_or_else(|| render_generated_head(name));

    if let Ok(path) = cache_path(uuid) {
        let _ = std::fs::write(path, &png);
    }
    to_data_url(&png)
}

/// 清除某个账号的头像缓存（换皮肤后想立刻刷新时用）。
pub fn clear_avatar_cache(uuid: &str) {
    if let Ok(path) = cache_path(uuid) {
        let _ = std::fs::remove_file(path);
    }
}

fn to_data_url(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

#[derive(Deserialize)]
struct ProfileResponse {
    #[serde(default)]
    properties: Vec<ProfileProperty>,
}

#[derive(Deserialize)]
struct ProfileProperty {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct TexturePayload {
    textures: TextureSet,
}

#[derive(Deserialize)]
struct TextureSet {
    #[serde(rename = "SKIN")]
    skin: Option<TextureEntry>,
}

#[derive(Deserialize)]
struct TextureEntry {
    url: String,
}

async fn fetch_official_head(uuid: &str) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;

    // uuid 带不带连字符 Mojang 都收，这里统一去掉连字符。
    let id = uuid.replace('-', "");
    let profile: ProfileResponse = client
        .get(format!("{PROFILE_URL}{id}"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    let encoded = profile
        .properties
        .iter()
        .find(|p| p.name == "textures")
        .map(|p| p.value.clone())?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let payload: TexturePayload = serde_json::from_slice(&decoded).ok()?;
    let skin_url = payload.textures.skin?.url;

    let skin = client.get(skin_url).send().await.ok()?.bytes().await.ok()?;
    head_from_skin(&skin)
}

/// 从整张皮肤里抠出 8x8 的头并放大到 64x64。
fn head_from_skin(bytes: &[u8]) -> Option<Vec<u8>> {
    let decoded = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = decoded.dimensions();
    if w < HEAD_X + 8 || h < HEAD_Y + 8 {
        return None;
    }

    let mut head = image::RgbaImage::new(8, 8);
    for y in 0..8 {
        for x in 0..8 {
            head.put_pixel(x, y, *decoded.get_pixel(HEAD_X + x, HEAD_Y + y));
        }
    }
    // 64x64 皮肤才有帽子层，有就叠上去，透明像素不覆盖。
    if w >= HAT_X + 8 && h >= HEAD_Y + 8 {
        for y in 0..8 {
            for x in 0..8 {
                let px = *decoded.get_pixel(HAT_X + x, HEAD_Y + y);
                if px[3] > 0 {
                    head.put_pixel(x, y, px);
                }
            }
        }
    }

    let scaled = image::imageops::resize(&head, 64, 64, image::imageops::FilterType::Nearest);
    encode_png(&scaled)
}

fn encode_png(img: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

/// 按用户名确定性生成一张 8x8 像素脸（肤色 / 发色 / 瞳色各不相同），
/// 放大到 64x64 后跟正版抠出来的头是一样的观感。
fn render_generated_head(seed: &str) -> Vec<u8> {
    const SKIN: [[u8; 3]; 6] = [
        [249, 201, 164],
        [232, 181, 140],
        [198, 138, 94],
        [238, 200, 170],
        [213, 158, 116],
        [168, 112, 74],
    ];
    const HAIR: [[u8; 3]; 7] = [
        [59, 42, 29],
        [107, 74, 47],
        [31, 26, 23],
        [168, 107, 50],
        [216, 176, 112],
        [122, 52, 44],
        [70, 70, 74],
    ];
    const EYES: [[u8; 3]; 4] = [[43, 43, 43], [59, 102, 165], [74, 138, 74], [110, 66, 40]];

    let mut state: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in seed.to_lowercase().bytes() {
        state ^= byte as u64;
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut next = |n: u64| -> u64 {
        // xorshift64*，够用且不引入依赖
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_f491_4f6c_dd1d) % n.max(1)
    };

    let skin = SKIN[next(SKIN.len() as u64) as usize];
    let hair = HAIR[next(HAIR.len() as u64) as usize];
    let eye = EYES[next(EYES.len() as u64) as usize];
    let long_hair = next(2) == 0;

    let mut img = image::RgbaImage::new(8, 8);
    for y in 0..8 {
        for x in 0..8 {
            img.put_pixel(x, y, image::Rgba([skin[0], skin[1], skin[2], 255]));
        }
    }

    let mut paint = |x: u32, y: u32, color: [u8; 3]| {
        if x < 8 && y < 8 {
            img.put_pixel(x, y, image::Rgba([color[0], color[1], color[2], 255]));
        }
    };

    // 刘海：前两行整排头发，第三行只留两侧鬓角
    for x in 0..8 {
        paint(x, 0, hair);
        paint(x, 1, hair);
    }
    paint(0, 2, hair);
    paint(7, 2, hair);
    if long_hair {
        paint(0, 3, hair);
        paint(7, 3, hair);
    }

    // 眼睛：第 4 行左右各两像素
    paint(1, 3, eye);
    paint(2, 3, eye);
    paint(5, 3, eye);
    paint(6, 3, eye);

    // 鼻影 + 嘴
    let shadow = [skin[0].saturating_sub(28), skin[1].saturating_sub(26), skin[2].saturating_sub(24)];
    paint(3, 4, shadow);
    paint(4, 4, shadow);
    paint(3, 5, shadow);
    paint(4, 5, shadow);

    let scaled = image::imageops::resize(&img, 64, 64, image::imageops::FilterType::Nearest);
    encode_png(&scaled).unwrap_or_default()
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// 造一张 64x64 假皮肤：头部整块蓝色，帽子层只点一个不透明绿像素。
    fn fake_skin() -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 0]));
        for y in 0..8 {
            for x in 0..8 {
                img.put_pixel(HEAD_X + x, HEAD_Y + y, image::Rgba([0, 0, 255, 255]));
            }
        }
        img.put_pixel(HAT_X, HEAD_Y, image::Rgba([0, 255, 0, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn head_is_cropped_and_hat_overlay_wins() {
        let png = head_from_skin(&fake_skin()).expect("皮肤应该能裁出头部");
        let head = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(head.dimensions(), (64, 64));
        // (0,0) 被帽子层覆盖成绿色
        assert_eq!(*head.get_pixel(0, 0), image::Rgba([0, 255, 0, 255]));
        // (1,0) 帽子层透明，露出底下的蓝色头部
        assert_eq!(*head.get_pixel(8, 0), image::Rgba([0, 0, 255, 255]));
    }

    #[test]
    fn generated_head_is_stable_png() {
        let a = render_generated_head("baby");
        let b = render_generated_head("baby");
        assert_eq!(a, b, "同一用户名的脸必须每次一致");
        let img = image::load_from_memory(&a).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (64, 64));
        assert_ne!(render_generated_head("baby"), render_generated_head("toxic"));
    }

    #[test]
    fn broken_skin_falls_back() {
        assert!(head_from_skin(b"not a png").is_none());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// 真机：拿真实正版账号的 uuid 问 Mojang 要皮肤，验证返回的是能解码的 64x64 头像。
    /// cargo test --lib live_official_skin -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_official_skin_avatar() {
        let uuid = "069a79f444e94726a5befca90e38aaf5"; // Notch
        let url = avatar_data_url(uuid, "Notch", AccountKind::Msa).await;
        assert!(url.starts_with("data:image/png;base64,"), "应该返回 data URL");
        let raw = url.split(',').nth(1).expect("data URL 里应有 base64 段");
        let png = base64::engine::general_purpose::STANDARD.decode(raw).unwrap();
        let img = image::load_from_memory(&png).expect("应能解码成图片").to_rgba8();
        println!("avatar bytes={} size={}x{}", png.len(), img.width(), img.height());
        assert_eq!(img.dimensions(), (64, 64));
        assert!(cache_path(uuid).unwrap().exists(), "应该落盘缓存");
    }

    /// 真机：随便一个不存在的 uuid 不能 panic，要退回本地生成的脸。
    #[tokio::test]
    #[ignore]
    async fn live_unknown_uuid_falls_back() {
        let url = avatar_data_url("00000000000000000000000000000000", "ghost", AccountKind::Msa).await;
        assert!(url.starts_with("data:image/png;base64,"));
        let _ = std::fs::remove_file(cache_path("00000000000000000000000000000000").unwrap());
    }
}
