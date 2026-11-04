//! PNG chunk 小工具：量化或无损重编码都会丢掉辅助 chunk，这里把需要保留的贴回输出。
//!
//! 同格式重编码保留色彩解释、DPI 与 EXIF。gAMA/cHRM 没有 iCCP 时仍然有效，不能丢。

pub const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// 重编码后要保留的辅助 chunk，按写入顺序排列。
pub const PRESERVED: [&[u8; 4]; 7] = [
    b"iCCP", b"sRGB", b"gAMA", b"cHRM", b"cICP", b"pHYs", b"eXIf",
];

/// 找第一个 IDAT 之前的 `kind` chunk，返回其内容（不含长度、类型、CRC）。
///
/// iCCP、pHYs、acTL 在 IDAT 前；eXIf 也允许在 IDAT 后，本函数不查找后者。
pub fn chunk_before_idat<'a>(png: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    find_chunk(png, kind, true)
}

fn find_chunk<'a>(png: &'a [u8], kind: &[u8; 4], before_idat: bool) -> Option<&'a [u8]> {
    if !png.starts_with(SIGNATURE) {
        return None;
    }
    let mut offset = SIGNATURE.len();
    while offset.checked_add(12)? <= png.len() {
        let length =
            u32::from_be_bytes(png.get(offset..offset.checked_add(4)?)?.try_into().ok()?) as usize;
        let end = offset.checked_add(12)?.checked_add(length)?;
        if end > png.len() {
            return None;
        }
        let current = png.get(offset + 4..offset + 8)?;
        if current == kind {
            return png.get(offset + 8..end - 4);
        }
        if current == b"IEND" || (before_idat && current == b"IDAT") {
            return None;
        }
        offset = end;
    }
    None
}

pub fn has_chunk(png: &[u8], kind: &[u8; 4]) -> bool {
    chunk_before_idat(png, kind).is_some()
}

/// 输出是否真的带着 iCCP。`kept_icc` 一律由它算出来，不再靠调用方传标志位——
/// 之前那个标志位在三条路径上都在说谎。
pub fn has_iccp(png: &[u8]) -> bool {
    has_chunk(png, b"iCCP")
}

/// 把 `source` 里 [`PRESERVED`] 列出的、`target` 里没有的 chunk 贴到 `target` 上。
///
/// 统一插在 IHDR 之后：iCCP 必须在 PLTE 之前、pHYs 和 eXIf 必须在 IDAT 之前，
/// IHDR 之后这一个位置同时满足三条，比「找 PLTE 或 IDAT」简单也不会错。
pub fn copy_preserved(source: &[u8], target: &[u8]) -> Vec<u8> {
    let Some(offset) = after_ihdr(target) else {
        return target.to_vec();
    };
    let mut insert = Vec::new();
    for kind in PRESERVED {
        if find_chunk(target, kind, false).is_some() {
            continue;
        }
        // eXIf 也允许出现在 IDAT 后，不能只看头部。
        let Some(payload) = find_chunk(source, kind, false) else {
            continue;
        };
        write_chunk(&mut insert, kind, payload);
    }
    if insert.is_empty() {
        return target.to_vec();
    }

    let mut out = Vec::with_capacity(target.len() + insert.len());
    out.extend_from_slice(&target[..offset]);
    out.extend_from_slice(&insert);
    out.extend_from_slice(&target[offset..]);
    out
}

/// IHDR 结束的位置，也就是第一个可以插辅助 chunk 的位置。
fn after_ihdr(png: &[u8]) -> Option<usize> {
    if png.len() < 33 || !png.starts_with(SIGNATURE) {
        return None;
    }
    let length = u32::from_be_bytes(png.get(8..12)?.try_into().ok()?) as usize;
    if png.get(12..16)? != b"IHDR" {
        return None;
    }
    let end = 8usize.checked_add(12)?.checked_add(length)?;
    (end <= png.len()).then_some(end)
}

pub fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out.extend_from_slice(&crc32(kind, payload).to_be_bytes());
}

/// PNG 用的 CRC-32（IEEE，反射式）。
fn crc32(kind: &[u8], payload: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in kind.iter().chain(payload) {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_chunk(&mut out, kind, payload);
        out
    }

    /// IHDR + 若干辅助 chunk + IDAT + IEND。IDAT 内容随意，只测 chunk 位置。
    fn png_with(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut png = SIGNATURE.to_vec();
        png.extend(chunk(b"IHDR", &[0u8; 13]));
        for (kind, payload) in chunks {
            png.extend(chunk(kind, payload));
        }
        png.extend(chunk(b"IDAT", b"pixels"));
        png.extend(chunk(b"IEND", b""));
        png
    }

    const PROFILE: &[u8] = b"profile\0\x00fake-icc";

    #[test]
    fn extracts_iccp_before_idat() {
        let png = png_with(&[(b"iCCP", PROFILE)]);
        assert_eq!(chunk_before_idat(&png, b"iCCP"), Some(PROFILE));
        assert!(has_iccp(&png));
    }

    #[test]
    fn no_iccp_returns_none() {
        let png = png_with(&[]);
        assert_eq!(chunk_before_idat(&png, b"iCCP"), None);
        assert!(!has_iccp(&png));
    }

    #[test]
    fn copies_all_preserved_chunks_after_ihdr() {
        let source = png_with(&[
            (b"pHYs", &[0, 0, 11, 19, 0, 0, 11, 19, 1]),
            (b"iCCP", PROFILE),
            (b"eXIf", b"exif-bytes"),
        ]);
        let target = png_with(&[]);

        let out = copy_preserved(&source, &target);
        for kind in [b"iCCP", b"pHYs", b"eXIf"] {
            assert!(
                has_chunk(&out, kind),
                "{} 应被搬过来",
                String::from_utf8_lossy(kind)
            );
        }
        assert_eq!(chunk_before_idat(&out, b"iCCP"), Some(PROFILE));
        assert_eq!(chunk_before_idat(&out, b"eXIf"), Some(&b"exif-bytes"[..]));
        // 搬过来的 chunk 不能排到 IHDR 前面
        assert_eq!(&out[12..16], b"IHDR");
    }

    #[test]
    fn iccp_lands_before_plte() {
        let source = png_with(&[(b"iCCP", PROFILE)]);
        let mut target = SIGNATURE.to_vec();
        target.extend(chunk(b"IHDR", &[0u8; 13]));
        target.extend(chunk(b"PLTE", &[0u8; 3]));
        target.extend(chunk(b"IDAT", b"pixels"));
        target.extend(chunk(b"IEND", b""));

        let out = copy_preserved(&source, &target);
        let iccp_at = out.windows(4).position(|w| w == b"iCCP").unwrap();
        let plte_at = out.windows(4).position(|w| w == b"PLTE").unwrap();
        assert!(iccp_at < plte_at, "iCCP 必须在 PLTE 之前");
    }

    #[test]
    fn existing_chunks_are_not_duplicated() {
        let source = png_with(&[(b"iCCP", PROFILE)]);
        let target = png_with(&[(b"iCCP", b"other-profile")]);
        assert_eq!(copy_preserved(&source, &target), target);
    }

    #[test]
    fn nothing_to_copy_is_a_noop() {
        let source = png_with(&[]);
        let target = png_with(&[]);
        assert_eq!(copy_preserved(&source, &target), target);
    }

    #[test]
    fn malformed_target_is_returned_as_is() {
        let source = png_with(&[(b"iCCP", PROFILE)]);
        let target = b"not a png at all".to_vec();
        assert_eq!(copy_preserved(&source, &target), target);
    }

    #[test]
    fn color_interpretation_without_icc_survives_reencoding() {
        let source = png_with(&[
            (b"gAMA", &100000u32.to_be_bytes()),
            (b"cHRM", &[1; 32]),
            (b"sRGB", &[0]),
        ]);
        let out = copy_preserved(&source, &png_with(&[]));
        for kind in [b"gAMA", b"cHRM", b"sRGB"] {
            assert_eq!(
                chunk_before_idat(&out, kind),
                chunk_before_idat(&source, kind)
            );
        }
    }
    #[test]
    fn exif_after_idat_is_preserved_and_truncated_chunks_are_ignored() {
        let mut source = png_with(&[]);
        source.truncate(source.len() - 12);
        source.extend(chunk(b"eXIf", b"exif-payload"));
        source.extend(chunk(b"IEND", b""));
        let out = copy_preserved(&source, &png_with(&[]));
        assert_eq!(
            chunk_before_idat(&out, b"eXIf"),
            Some(b"exif-payload".as_slice())
        );
        let mut broken = SIGNATURE.to_vec();
        broken.extend_from_slice(&u32::MAX.to_be_bytes());
        broken.extend_from_slice(b"iCCP");
        assert_eq!(chunk_before_idat(&broken, b"iCCP"), None);
    }

    #[test]
    fn crc_matches_known_value() {
        // IEND 的 CRC 是 PNG 规范里写死的常量。
        assert_eq!(crc32(b"IEND", b""), 0xAE42_6082);
    }
}
