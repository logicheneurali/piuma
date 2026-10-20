//! Bar colours derived from the wallpaper at runtime: the panel's
//! `wallpaper_palette` option, applied over the configured defaults.
//!
//! The extracted colours are NEVER persisted — the config file only
//! carries the toggle and this palette lives in the bar's memory — so
//! switching the option (or the wallpaper) off drops it and the next
//! rebuild resolves the user's own colours again. Extraction runs off
//! the UI thread and only when the wallpaper file changes (path or
//! mtime), never per tick.

use std::path::Path;

use crate::config;

/// The derived tones as the `#rrggbb` strings the config colour chains
/// already parse: panel background, sensor default card (its hover
/// variant is tinted with the most vibrant hue) and text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: String,
    pub card: String,
    pub card_hover: String,
    pub text: String,
    pub text_hover: String,
}

/// The tone with the CONFIGURED colour's alpha when it isn't opaque: a
/// transparent config stays transparent in the wallpaper's hue (a
/// `#15306000` section card becomes the tone with `a = 0`). An absent,
/// unparsable or opaque configured colour leaves the tone as-is.
pub fn keep_config_alpha(tone: &str, configured: Option<&str>) -> String {
    let Some(alpha) = configured
        .and_then(|c| config::parse_color(c).ok())
        .map(|c| c.a)
        .filter(|a| *a < 1.0)
    else {
        return tone.to_string();
    };
    match config::parse_color(tone) {
        Ok(mut c) => {
            c.a = alpha;
            let byte = |v: f32| (v * 255.0).round() as u8;
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                byte(c.r),
                byte(c.g),
                byte(c.b),
                byte(c.a)
            )
        }
        // A tone that does not parse is left as the caller passed it (a
        // load-time log names the value; rendering must not break).
        Err(_) => tone.to_string(),
    }
}

/// Longest edge of the extraction thumbnail: a 4K wallpaper decodes in
/// milliseconds and the small histogram stays stable.
const THUMB: u32 = 64;

/// Hue bins (30° wide): one bucket per bin plus one for greyish pixels.
const HUE_BINS: usize = 12;

/// Saturation floor separating a tinted bucket from a grey one.
const SAT_MIN: f32 = 0.25;

/// Minimum population (pixel count and share of the thumbnail) for a
/// bucket to be considered vibrant: a handful of vivid pixels must not
/// drive the hover colour.
const VIBRANT_MIN: u32 = 8;
const VIBRANT_SHARE: u32 = 50;

/// Target HSL lightness of the derived tones: a dark bar, cards a step
/// above the panel, hover a step further, low-saturation text.
const L_BG: f32 = 0.10;
const L_CARD: f32 = 0.19;
const L_CARD_HOVER: f32 = 0.30;
const L_TEXT: f32 = 0.85;
const L_TEXT_HOVER: f32 = 0.96;

/// Decode the wallpaper file and derive its palette; `None` when the
/// file can't be decoded (the caller keeps the user's own colours).
pub fn extract(path: &Path) -> Option<Palette> {
    let img = image::open(path).ok()?.thumbnail(THUMB, THUMB);
    from_rgba(&img.to_rgba8())
}

/// Pure derivation over RGBA pixels: the dominant bucket sets the base
/// tones, the most vibrant bucket (enough saturation and population)
/// tints the hover card. Near-transparent pixels are skipped; an
/// all-transparent image yields `None`.
pub fn from_rgba(img: &image::RgbaImage) -> Option<Palette> {
    // Per bucket (one per hue bin, plus a grey one): pixel count, summed
    // 8-bit rgb and summed saturation (to pick the vibrant).
    let mut buckets: Vec<(u32, f32, f32, f32, f32)> = vec![(0, 0.0, 0.0, 0.0, 0.0); HUE_BINS + 1];
    let mut visible = 0u32;
    for p in img.pixels() {
        if p.0[3] < 128 {
            continue;
        }
        let (h, s, _) = rgb_to_hsl(f32::from(p.0[0]), f32::from(p.0[1]), f32::from(p.0[2]));
        let idx = if s < SAT_MIN {
            HUE_BINS
        } else {
            ((h * HUE_BINS as f32) as usize).min(HUE_BINS - 1)
        };
        let b = &mut buckets[idx];
        b.0 += 1;
        b.1 += f32::from(p.0[0]);
        b.2 += f32::from(p.0[1]);
        b.3 += f32::from(p.0[2]);
        b.4 += s;
        visible += 1;
    }
    if visible == 0 {
        return None;
    }
    let mean = |i: usize| {
        let b = &buckets[i];
        (
            b.1 / b.0 as f32,
            b.2 / b.0 as f32,
            b.3 / b.0 as f32,
            b.4 / b.0 as f32,
        )
    };
    // Dominant: the biggest bucket (greys included — a dark wallpaper
    // must still produce a calm bar).
    let dominant = (0..=HUE_BINS).max_by_key(|&i| buckets[i].0)?;
    // Vibrant: the best saturation × sqrt(population) among populated
    // tinted buckets, skipping the dominant one when another candidate
    // exists — the hover accent adds a distinct tint instead of
    // repeating the base. Grey/single-hue images fall back to it.
    let min_pop = VIBRANT_MIN.max(visible / VIBRANT_SHARE);
    let score = |i: usize| mean(i).3 * (buckets[i].0 as f32).sqrt();
    let candidates = |skip: Option<usize>| {
        (0..HUE_BINS)
            .filter(|&i| Some(i) != skip && buckets[i].0 >= min_pop)
            .max_by(|&a, &b| score(a).total_cmp(&score(b)))
    };
    let vibrant = candidates(Some(dominant))
        .or_else(|| candidates(None))
        .unwrap_or(dominant);
    let (dr, dg, db, _) = mean(dominant);
    let (vr, vg, vb, _) = mean(vibrant);
    let (dh, ds, _) = rgb_to_hsl(dr, dg, db);
    let (vh, vs, _) = rgb_to_hsl(vr, vg, vb);
    Some(Palette {
        bg: hex(hsl_to_rgb(dh, ds, L_BG)),
        card: hex(hsl_to_rgb(dh, ds, L_CARD)),
        card_hover: hex(hsl_to_rgb(vh, vs.max(SAT_MIN), L_CARD_HOVER)),
        text: hex(hsl_to_rgb(dh, (ds * 0.3).min(0.18), L_TEXT)),
        text_hover: hex(hsl_to_rgb(dh, (ds * 0.2).min(0.12), L_TEXT_HOVER)),
    })
}

/// HSL of an 8-bit-per-channel RGB triple; every component 0..1.
fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

/// The inverse of [`rgb_to_hsl`]: 8-bit channels out.
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let hue2rgb = |p: f32, q: f32, t: f32| {
        let mut t = t;
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        c(hue2rgb(p, q, h + 1.0 / 3.0)),
        c(hue2rgb(p, q, h)),
        c(hue2rgb(p, q, h - 1.0 / 3.0)),
    )
}

/// `#rrggbb` of an 8-bit RGB triple.
fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lightness of a `#rrggbb` string (HSL's `l`).
    fn lightness(hex: &str) -> f32 {
        let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
        let (r, g, b) = (
            ((v >> 16) & 255) as f32,
            ((v >> 8) & 255) as f32,
            (v & 255) as f32,
        );
        rgb_to_hsl(r, g, b).2
    }

    /// 8-bit RGB of a `#rrggbb` string.
    fn components(hex: &str) -> (u8, u8, u8) {
        let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
        (
            ((v >> 16) & 255) as u8,
            ((v >> 8) & 255) as u8,
            (v & 255) as u8,
        )
    }

    /// A blue-dominant image with a vivid red patch: the base tones stay
    /// blue, the hover card follows the vibrant red, and the lightness
    /// ladder (bg < card < hover < text < text_hover) holds.
    #[test]
    fn follows_the_dominant_and_the_vibrant_hue() {
        let mut img = image::RgbaImage::from_pixel(32, 32, image::Rgba([0, 0, 0, 0]));
        for (x, _, p) in img.enumerate_pixels_mut() {
            let c = if x < 22 {
                [13, 26, 92, 255]
            } else {
                [220, 26, 26, 255]
            };
            *p = image::Rgba(c);
        }
        let pal = from_rgba(&img).expect("palette");
        let (cr, _, cb) = components(&pal.card);
        assert!(cb > cr, "card should stay blue: {pal:?}");
        let (hr, _, hb) = components(&pal.card_hover);
        assert!(hr > hb, "hover should be red: {pal:?}");
        assert!(lightness(&pal.bg) < lightness(&pal.card), "{pal:?}");
        assert!(lightness(&pal.card) < lightness(&pal.card_hover), "{pal:?}");
        assert!(lightness(&pal.card_hover) < lightness(&pal.text), "{pal:?}");
        assert!(lightness(&pal.text) < lightness(&pal.text_hover), "{pal:?}");
    }

    /// An all-transparent image has nothing to derive.
    #[test]
    fn skips_images_without_visible_pixels() {
        let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 0]));
        assert!(from_rgba(&img).is_none());
    }

    /// A grey wallpaper still yields the tone ladder (no hue to follow).
    #[test]
    fn grey_images_keep_the_tone_ladder() {
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([128, 128, 128, 255]));
        let pal = from_rgba(&img).expect("palette");
        assert!(lightness(&pal.bg) < lightness(&pal.card), "{pal:?}");
        assert!(lightness(&pal.card) < lightness(&pal.text), "{pal:?}");
    }

    /// A missing file yields `None` (the caller keeps the user's colours).
    #[test]
    fn missing_file_extracts_nothing() {
        assert!(extract(Path::new("/nonexistent/piuma-wallpaper.png")).is_none());
    }

    /// The palette's tone keeps the configured colour's alpha when it
    /// isn't opaque (`#15306000` → the tone with `a = 0`); an opaque,
    /// absent or unparsable configured colour leaves the tone as-is.
    #[test]
    fn keep_config_alpha_preserves_the_config_transparency() {
        assert_eq!(keep_config_alpha("#000061", Some("#15306000")), "#00006100");
        assert_eq!(keep_config_alpha("#000061", Some("#15306080")), "#00006180");
        assert_eq!(keep_config_alpha("#000061", Some("#153060")), "#000061");
        assert_eq!(keep_config_alpha("#000061", None), "#000061");
        assert_eq!(keep_config_alpha("#000061", Some("junk")), "#000061");
    }
}
