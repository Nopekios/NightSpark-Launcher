//! 哈希与文件校验。
//!
//! 这里自己实现 SHA-1，不引第三方 crate：整合包下载链路只需要这一个算法，
//! 手写一份能保证行为可读、可单测，也免得为一个 hash 拖一个依赖进来。

use std::path::Path;

use crate::error::AppError;

/// 对一段字节算 SHA-1，返回 20 字节摘要。
fn sha1_digest(bytes: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];

    // 补位：先 0x80，再补 0 到 56 (mod 64)，最后 8 字节大端存比特长度。
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(bytes.len() + 72);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 80];
        for index in 0..16 {
            let start = index * 4;
            words[index] = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for index in 16..80 {
            words[index] = (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                .rotate_left(1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];

        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDCu32),
                _ => (b ^ c ^ d, 0xCA62_C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for (index, value) in state.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    out
}

/// 把摘要转成小写十六进制串。
fn to_hex(digest: &[u8]) -> String {
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// 对内存里的内容算 SHA-1 十六进制串。
pub fn sha1_bytes(bytes: &[u8]) -> String {
    to_hex(&sha1_digest(bytes))
}

/// 对磁盘文件算 SHA-1 十六进制串。
pub fn sha1_file(path: &Path) -> Result<String, AppError> {
    let bytes = std::fs::read(path)?;
    Ok(sha1_bytes(&bytes))
}

/// 判断一条哈希串是什么算法：40 位是 SHA-1，32 位是 MD5，64 位是 SHA-256。
///
/// 目前只对 SHA-1 做真实校验；别的算法返回 `None`，调用方按「无法校验」放行并留提示，
/// 而不是伪造一个「已校验」的结论。
pub fn supported_sha1(expected: &str) -> Option<&str> {
    let trimmed = expected.trim();
    if trimmed.len() == 40 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(trimmed)
    } else {
        None
    }
}

/// 校验文件哈希是否与期望一致（大小写不敏感）。
pub fn verify_file(path: &Path, expected: &str) -> Result<bool, AppError> {
    let Some(expected) = supported_sha1(expected) else {
        return Ok(false);
    };
    let actual = sha1_file(path)?;
    Ok(actual.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用标准向量确认 SHA-1 实现没写歪。
    #[test]
    fn probe_sha1_known_vectors() {
        assert_eq!(sha1_bytes(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_bytes(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_bytes(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // 跨过 64 字节分块边界，顺便盯一下补位逻辑。
        let long = vec![b'a'; 1_000_000];
        assert_eq!(sha1_bytes(&long), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }

    /// 哈希串形态判断：只有 40 位十六进制才认为可以校验。
    #[test]
    fn probe_supported_sha1() {
        assert!(supported_sha1("a9993e364706816aba3e25717850c26c9cd0d89d").is_some());
        assert!(supported_sha1("A9993E364706816ABA3E25717850C26C9CD0D89D").is_some());
        assert!(supported_sha1("d41d8cd98f00b204e9800998ecf8427e").is_none());
        assert!(supported_sha1("").is_none());
    }
}
