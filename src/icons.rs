//! The tray icons, drawn in code (no image assets, no icon-theme dependency).
//!
//! Each icon is a magnifier glyph in the panel's foreground tone plus a status
//! dot: green = running, amber = transitioning, blue = updating, grey = stopped.
//! Rendering is supersampled 4x and box-downsampled, which is all the
//! anti-aliasing a 22px tray icon needs.

use ksni::Icon;

use crate::state::RunState;

const SS: usize = 4;
const SIZES: [usize; 2] = [22, 44];

/// Panel tone, read from `~/.config/kdeglobals` (`[Colors:Window]
/// BackgroundNormal`), which Plasma rewrites on every colour-scheme change.
pub fn theme_is_dark() -> Option<bool> {
    let home = std::env::var_os("HOME")?;
    let text =
        std::fs::read_to_string(std::path::Path::new(&home).join(".config/kdeglobals")).ok()?;
    let mut in_window = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_window = line == "[Colors:Window]";
            continue;
        }
        if !in_window {
            continue;
        }
        if let Some(value) = line.strip_prefix("BackgroundNormal=") {
            let rgb: Vec<f32> = value
                .split(',')
                .filter_map(|p| p.trim().parse::<f32>().ok())
                .take(3)
                .collect();
            if rgb.len() == 3 {
                let luminance = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
                return Some(luminance < 128.0);
            }
        }
    }
    None
}

pub fn build_icons(state: RunState, dark: bool) -> Vec<Icon> {
    SIZES
        .iter()
        .map(|&size| render(size, state, dark))
        .collect()
}

/// State colour for the status dot. These are the *theme's* own emblem colours
/// (`emblem-success` #27ae60, `emblem-unavailable` #7f8c8d, `emblem-warning`
/// #f67400, `emblem-synchronizing` #00acc1), which the menu header uses for the
/// same states — so the panel icon and the open menu name the same colour. They
/// read on both light and dark panels, so no per-theme variant is needed.
fn state_color(state: RunState) -> [f32; 3] {
    scale(match state {
        RunState::Running => [39, 174, 96],
        RunState::Stopped => [127, 140, 141],
        RunState::Updating => [0, 172, 193],
        RunState::Starting | RunState::Stopping => [246, 116, 0],
    })
}

fn glyph_color(dark: bool) -> [f32; 3] {
    scale(if dark { [239, 240, 241] } else { [35, 38, 41] })
}

fn scale(rgb: [u8; 3]) -> [f32; 3] {
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    ]
}

fn render(size: usize, state: RunState, dark: bool) -> Icon {
    let n = (size * SS) as f32;
    let mut canvas = Canvas::new(size * SS);

    let ink = glyph_color(dark);

    // 10% padding: the magnifier lives inside [pad, 1-pad] so it breathes in the
    // panel instead of filling the whole canvas. The handle runs to the
    // bottom-right corner of that box; the status dot sits in the bottom-left
    // corner, opposite the handle, where the two can never collide.
    let pad = n * 0.10;
    let box_size = n - 2.0 * pad;
    let lens_r = box_size * 0.30;
    let stroke = box_size * 0.10;
    let lens_cx = pad + box_size * 0.45;
    let lens_cy = pad + box_size * 0.375;

    // Lens ring.
    canvas.paint(lens_cx, lens_cy, lens_r + stroke, ink, |d| {
        (d - lens_r).abs() <= stroke / 2.0
    });
    // Handle, at 45 degrees down-right.
    let a = std::f32::consts::FRAC_1_SQRT_2;
    canvas.capsule(
        lens_cx + lens_r * a,
        lens_cy + lens_r * a,
        pad + box_size * 0.95,
        pad + box_size * 0.875,
        stroke * 1.1,
        ink,
    );

    // Status dot: slightly larger than the glyph stroke so it reads as a badge
    // at 22px, pulled far enough out to leave a pixel of background between it
    // and the lens ring.
    canvas.circle(
        pad + box_size * 0.09,
        pad + box_size * 0.86,
        box_size * 0.20,
        state_color(state),
    );

    downsample(&canvas, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// '.' background, '#' glyph ink, 's' the grey stopped dot, 'G'/'Y'/'B' the
    /// coloured states.
    fn classify(a: i32, r: i32, g: i32, b: i32) -> char {
        if a < 40 {
            '.'
        } else if g > 90 && g > r + 30 && g > b + 30 {
            'G'
        } else if r > 150 && (60..160).contains(&g) && b < 90 {
            'Y'
        } else if b > 120 && b > r + 30 && b > g + 10 {
            'B'
        } else if r >= 200 {
            '#' // light glyph ink on a dark panel
        } else if r >= 110 {
            's' // the muted grey of the stopped dot
        } else {
            '%' // dark glyph ink on a light panel
        }
    }

    fn map(state: RunState, dark: bool) -> String {
        let icon = render(22, state, dark);
        let mut out = String::new();
        for y in 0..icon.height {
            for x in 0..icon.width {
                let i = ((y * icon.width + x) * 4) as usize;
                out.push(classify(
                    i32::from(icon.data[i]),
                    i32::from(icon.data[i + 1]),
                    i32::from(icon.data[i + 2]),
                    i32::from(icon.data[i + 3]),
                ));
            }
            out.push('\n');
        }
        out
    }

    fn is_dot(ch: char) -> bool {
        matches!(ch, 'G' | 'Y' | 'B' | 's')
    }

    #[test]
    fn icon_geometry_is_legible() {
        for state in [
            RunState::Running,
            RunState::Stopped,
            RunState::Starting,
            RunState::Updating,
        ] {
            let art = map(state, true);
            println!("--- {state:?} (22px, dark panel) ---\n{art}");
            let grid: Vec<Vec<char>> = art.lines().map(|l| l.chars().collect()).collect();

            // The lens is a ring, not a disc: its centre is background.
            assert_eq!(grid[9][10], '.', "{state:?}: lens centre is filled");

            // The handle runs into the bottom-right quadrant.
            let handle_ink = (12..22)
                .flat_map(|y| (14..22).map(move |x| (y, x)))
                .filter(|&(y, x)| grid[y][x] == '#')
                .count();
            assert!(
                handle_ink >= 3,
                "{state:?}: no handle in the bottom-right ({handle_ink} px)"
            );

            // The status dot is a block of state colour in the bottom-left.
            let dots: Vec<(usize, usize)> = (12..22)
                .flat_map(|y| (0..8).map(move |x| (y, x)))
                .filter(|&(y, x)| is_dot(grid[y][x]))
                .collect();
            assert!(
                dots.len() >= 20,
                "{state:?}: status dot too small ({} px)",
                dots.len()
            );

            // ...and it never touches the glyph: no dot pixel has an ink
            // neighbour (the whole point of moving it to the opposite corner
            // from the handle).
            for &(y, x) in &dots {
                let neighbours = [
                    (y.wrapping_sub(1), x),
                    (y + 1, x),
                    (y, x.wrapping_sub(1)),
                    (y, x + 1),
                ];
                for (ny, nx) in neighbours {
                    if ny < 22 && nx < 22 && grid[ny][nx] == '#' {
                        panic!("{state:?}: status dot touches the glyph at ({y},{x})-({ny},{nx})");
                    }
                }
            }
        }
    }
}

struct Canvas {
    w: usize,
    /// Premultiplied RGBA.
    data: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(w: usize) -> Self {
        Self {
            w,
            data: vec![[0.0; 4]; w * w],
        }
    }

    fn blend(&mut self, x: usize, y: usize, rgb: [f32; 3]) {
        let i = y * self.w + x;
        let dst = self.data[i];
        let src_a = 1.0f32;
        let out_a = src_a + dst[3] * (1.0 - src_a);
        self.data[i] = if out_a <= 0.0 {
            [0.0; 4]
        } else {
            [
                rgb[0] * src_a + dst[0] * (1.0 - src_a),
                rgb[1] * src_a + dst[1] * (1.0 - src_a),
                rgb[2] * src_a + dst[2] * (1.0 - src_a),
                out_a,
            ]
        };
    }

    fn paint<F: Fn(f32) -> bool>(&mut self, cx: f32, cy: f32, r: f32, rgb: [f32; 3], inside: F) {
        let x0 = (cx - r).floor().max(0.0) as usize;
        let x1 = ((cx + r).ceil().min(self.w as f32 - 1.0)).max(0.0) as usize;
        let y0 = (cy - r).floor().max(0.0) as usize;
        let y1 = ((cy + r).ceil().min(self.w as f32 - 1.0)).max(0.0) as usize;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                if inside(d) {
                    self.blend(x, y, rgb);
                }
            }
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, rgb: [f32; 3]) {
        self.paint(cx, cy, r, rgb, |d| d <= r);
    }

    /// A rounded band from (x1,y1) to (x2,y2) of the given thickness.
    fn capsule(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, rgb: [f32; 3]) {
        let half = thickness / 2.0;
        let cx = (x1 + x2) / 2.0;
        let cy = (y1 + y2) / 2.0;
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len2 = (dx * dx + dy * dy).max(f32::EPSILON);
        let reach = len2.sqrt() / 2.0 + half;
        let x0 = (cx - reach).floor().max(0.0) as usize;
        let x1i = ((cx + reach).ceil().min(self.w as f32 - 1.0)).max(0.0) as usize;
        let y0 = (cy - reach).floor().max(0.0) as usize;
        let y1i = ((cy + reach).ceil().min(self.w as f32 - 1.0)).max(0.0) as usize;
        for y in y0..=y1i {
            for x in x0..=x1i {
                let px = x as f32 + 0.5 - cx;
                let py = y as f32 + 0.5 - cy;
                let t = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
                let nx = px - t * dx;
                let ny = py - t * dy;
                if (nx * nx + ny * ny).sqrt() <= half {
                    self.blend(x, y, rgb);
                }
            }
        }
    }
}

fn downsample(canvas: &Canvas, size: usize) -> Icon {
    let mut data = Vec::with_capacity(size * size * 4);
    let samples = (SS * SS) as f32;
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = canvas.data[(y * SS + sy) * canvas.w + (x * SS + sx)];
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                    acc[3] += p[3];
                }
            }
            let alpha = acc[3] / samples;
            let channel = |v: f32| -> u8 {
                let straight = if alpha > 0.0 {
                    v / samples / alpha
                } else {
                    0.0
                };
                (straight * 255.0).round().clamp(0.0, 255.0) as u8
            };
            // ARGB32, network byte order.
            data.push((alpha * 255.0).round().clamp(0.0, 255.0) as u8);
            data.push(channel(acc[0]));
            data.push(channel(acc[1]));
            data.push(channel(acc[2]));
        }
    }
    Icon {
        width: size as i32,
        height: size as i32,
        data,
    }
}
