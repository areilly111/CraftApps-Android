//! Tiny seven-segment digit renderer (for burn-ins and the counting leader; no font needed).

/// Segment masks for 0-9, then ':' (10), '-' (11), ';' (12).
const SEGS: [u8; 10] = [0b0111111, 0b0000110, 0b1011011, 0b1001111, 0b1100110, 0b1101101, 0b1111101, 0b0000111, 0b1111111, 0b1101111];

/// Draw `text` (digits, ':' ';' '-' and ' ') into an RGBA8 buffer at (x, y) with digit height `h`.
pub fn draw_text(buf: &mut [u8], width: usize, height: usize, x: i32, y: i32, h: i32, text: &str, color: [u8; 4]) {
    let w = h / 2;
    let t = (h / 10).max(1);
    let mut cx = x;
    for ch in text.chars() {
        match ch {
            '0'..='9' => {
                let m = SEGS[(ch as u8 - b'0') as usize];
                let half = h / 2;
                let segs: [(i32, i32, i32, i32); 7] = [
                    (cx, y, w, t),                   // a top
                    (cx + w - t, y, t, half),        // b upper right
                    (cx + w - t, y + half, t, half), // c lower right
                    (cx, y + h - t, w, t),           // d bottom
                    (cx, y + half, t, half),         // e lower left
                    (cx, y, t, half),                // f upper left
                    (cx, y + half - t / 2, w, t),    // g middle
                ];
                for (i, r) in segs.iter().enumerate() {
                    if m & (1 << i) != 0 {
                        fill_rect(buf, width, height, r.0, r.1, r.2, r.3, color);
                    }
                }
                cx += w + t * 2;
            }
            ':' | ';' => {
                fill_rect(buf, width, height, cx, y + h / 4, t, t, color);
                let y2 = y + h * 3 / 4 - t;
                fill_rect(buf, width, height, cx, y2, t, if ch == ';' { t * 2 } else { t }, color);
                cx += t * 3;
            }
            '-' => {
                fill_rect(buf, width, height, cx, y + h / 2 - t / 2, w, t, color);
                cx += w + t * 2;
            }
            _ => cx += w + t * 2,
        }
    }
}

/// Width in pixels that [`draw_text`] will use.
pub fn text_width(h: i32, text: &str) -> i32 {
    let w = h / 2;
    let t = (h / 10).max(1);
    text.chars().map(|c| if c == ':' || c == ';' { t * 3 } else { w + t * 2 }).sum()
}

pub fn fill_rect(buf: &mut [u8], width: usize, height: usize, x: i32, y: i32, w: i32, h: i32, c: [u8; 4]) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = ((x + w).max(0) as usize).min(width);
    let y1 = ((y + h).max(0) as usize).min(height);
    for yy in y0..y1 {
        for xx in x0..x1 {
            let i = (yy * width + xx) * 4;
            let a = c[3] as u32;
            for k in 0..3 {
                buf[i + k] = ((buf[i + k] as u32 * (255 - a) + c[k] as u32 * a) / 255) as u8;
            }
            buf[i + 3] = buf[i + 3].max(c[3]);
        }
    }
}
