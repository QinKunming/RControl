//! 图像转换 + JPEG 编码

/// BGRA (top-down) -> RGB
pub fn bgra_to_rgb(bgra: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(bgra.len() / 4 * 3);
    let mut i = 0;
    while i + 3 < bgra.len() + 1 && i + 3 < bgra.len() {
        rgb.push(bgra[i + 2]);
        rgb.push(bgra[i + 1]);
        rgb.push(bgra[i]);
        i += 4;
    }
    rgb
}

/// BGRA -> RGB 并按目标尺寸缩小 (box filter 平均, 视觉质量足够)
pub fn bgra_scale_to_rgb(bgra: &[u8], sw: u32, sh: u32, tw: u32, th: u32) -> Vec<u8> {
    if tw == 0 || th == 0 || sw == 0 || sh == 0 {
        return Vec::new();
    }
    let mut out = vec![0u8; (tw * th * 3) as usize];
    let x_ratio = sw as f64 / tw as f64;
    let y_ratio = sh as f64 / th as f64;
    for ty in 0..th {
        let y0 = (ty as f64 * y_ratio) as u32;
        let y1 = (((ty + 1) as f64 * y_ratio).ceil() as u32).clamp(y0 + 1, sh);
        for tx in 0..tw {
            let x0 = (tx as f64 * x_ratio) as u32;
            let x1 = (((tx + 1) as f64 * x_ratio).ceil() as u32).clamp(x0 + 1, sw);
            let mut r = 0u64;
            let mut g = 0u64;
            let mut b = 0u64;
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let idx = ((y * sw + x) * 4) as usize;
                    b += bgra[idx] as u64;
                    g += bgra[idx + 1] as u64;
                    r += bgra[idx + 2] as u64;
                    n += 1;
                }
            }
            let o = ((ty * tw + tx) * 3) as usize;
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
        }
    }
    out
}

/// RGB -> JPEG
pub fn encode_jpeg(rgb: &[u8], w: u32, h: u32, quality: u32) -> Option<Vec<u8>> {
    let q = quality.clamp(1, 100) as u8;
    if w == 0 || h == 0 || w > 65535 || h > 65535 {
        return None;
    }
    let mut out = Vec::with_capacity(rgb.len() / 8);
    {
        let enc = jpeg_encoder::Encoder::new(&mut out, q);
        enc.encode(rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).ok()?;
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_and_encode() {
        let (w, h) = (64u32, 32u32);
        let bgra: Vec<u8> = (0..(w * h * 4) as usize).map(|i| (i % 251) as u8).collect();
        let rgb = bgra_to_rgb(&bgra);
        assert_eq!(rgb.len(), (w * h * 3) as usize);
        let jpg = encode_jpeg(&rgb, w, h, 80).unwrap();
        assert!(!jpg.is_empty());
        let scaled = bgra_scale_to_rgb(&bgra, w, h, 32, 16);
        assert_eq!(scaled.len(), 32 * 16 * 3);
    }

    #[test]
    fn encode_4k_timing() {
        // 4K 编码耗时: 噪声(最坏情况) + 平滑内容(接近真实桌面)
        let (w, h) = (3840u32, 2160u32);
        let noise: Vec<u8> = (0..(w * h * 3) as usize).map(|i| ((i * 31 + i / 4096) % 253) as u8).collect();
        let t0 = std::time::Instant::now();
        let jpg = encode_jpeg(&noise, w, h, 70).unwrap();
        println!("4K noise encode: {} ms, {} bytes", t0.elapsed().as_millis(), jpg.len());
        // 平滑渐变 + 周期性矩形 (模拟桌面: 大片纯色 + 文字边缘)
        let smooth: Vec<u8> = (0..h).flat_map(|y| {
            (0..w).flat_map(move |x| {
                let base = ((x / 16) % 8) * 24 + ((y / 16) % 8) * 17;
                vec![base as u8, (base + ((x % 16) / 8) * 8) as u8, (base + 40) as u8]
            })
        }).collect();
        let t0 = std::time::Instant::now();
        let jpg = encode_jpeg(&smooth, w, h, 70).unwrap();
        let ms = t0.elapsed().as_millis();
        println!("4K smooth encode: {} ms, {} bytes", ms, jpg.len());
        assert!(ms < 1500, "4K encode too slow: {}ms", ms);
    }
}
