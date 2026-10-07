//! Pixel width measurement for pinned sensor text (roadmap M5).
//!
//! cosmic-text is the very shaper iced renders with (iced_graphics drives it
//! directly), so a width measured here equals the drawn width for whatever
//! font, weight and size the config selects — no assumption about a specific
//! font's figures (tabular or not).

use iced::font::{Family, Font, Weight};
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, PoisonError};

/// Width of the longest measured sample, in px: any text that far can't be
/// reached in practice, it only exists to disable line wrapping.
const UNBOUNDED: f32 = 100_000.0;

/// Result of the last measurement: the sections are rebuilt on every tick,
/// so the system font scan must happen only when font/size really change.
static CACHE: Mutex<Option<CacheKey>> = Mutex::new(None);

/// Identity of a measured width: font, size AND the pattern list (a
/// different list is a different question and must be measured again).
struct CacheKey {
    family: String,
    weight: Weight,
    size: u32,
    patterns: u64,
    width: u16,
}

/// Widest pixel width among `patterns`, shaped with `font` at `font_size` px.
/// Both shaping modes iced may use are measured and the larger result wins,
/// so the pin can never be narrower than what gets drawn; fractional widths
/// round up (a sub-pixel under-measure would make iced wrap the text).
pub fn widest(font: &Font, font_size: u32, patterns: &[String]) -> u16 {
    let family = format!("{:?}", font.family);
    let mut hasher = std::hash::DefaultHasher::new();
    for pattern in patterns {
        pattern.hash(&mut hasher);
    }
    let patterns_hash = hasher.finish();
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(hit) = cache.as_ref().filter(|k| {
        k.family == family
            && k.weight == font.weight
            && k.size == font_size
            && k.patterns == patterns_hash
    }) {
        return hit.width;
    }
    let width = measure(font, font_size, patterns);
    *cache = Some(CacheKey {
        family,
        weight: font.weight,
        size: font_size,
        patterns: patterns_hash,
        width,
    });
    width
}

/// Shape every pattern in both modes and return the maximum line width.
/// A [`cosmic_text::FontSystem`] is built locally and dropped right after:
/// fonts are memory-mapped, the scan costs one-off (the result is cached).
fn measure(font: &Font, font_size: u32, patterns: &[String]) -> u16 {
    let mut font_system = cosmic_text::FontSystem::new();
    let attrs = cosmic_text::Attrs {
        family: match font.family {
            Family::Name(name) => cosmic_text::Family::Name(name),
            Family::SansSerif => cosmic_text::Family::SansSerif,
            Family::Serif => cosmic_text::Family::Serif,
            Family::Cursive => cosmic_text::Family::Cursive,
            Family::Fantasy => cosmic_text::Family::Fantasy,
            Family::Monospace => cosmic_text::Family::Monospace,
        },
        weight: to_cosmic_weight(font.weight),
        ..cosmic_text::Attrs::new()
    };
    let mut widest = 0.0_f32;
    for pattern in patterns {
        for shaping in [cosmic_text::Shaping::Basic, cosmic_text::Shaping::Advanced] {
            let mut buffer = cosmic_text::Buffer::new(
                &mut font_system,
                cosmic_text::Metrics::new(font_size as f32, font_size as f32),
            );
            // Single unbounded line: the width of the whole text, never wrapped.
            buffer.set_size(&mut font_system, Some(UNBOUNDED), Some(UNBOUNDED));
            buffer.set_wrap(&mut font_system, cosmic_text::Wrap::None);
            let _ = buffer.set_text(&mut font_system, pattern, &attrs, shaping, None);
            widest = widest.max(
                buffer
                    .layout_runs()
                    .map(|run| run.line_w)
                    .fold(0.0_f32, f32::max),
            );
        }
    }
    widest.ceil().min(u16::MAX as f32) as u16
}

/// Map iced's weight enum onto fontdb's numeric weights (re-exported by
/// cosmic-text as a struct with constants).
fn to_cosmic_weight(weight: Weight) -> cosmic_text::Weight {
    match weight {
        Weight::Thin => cosmic_text::Weight::THIN,
        Weight::ExtraLight => cosmic_text::Weight::EXTRA_LIGHT,
        Weight::Light => cosmic_text::Weight::LIGHT,
        Weight::Normal => cosmic_text::Weight::NORMAL,
        Weight::Medium => cosmic_text::Weight::MEDIUM,
        Weight::Semibold => cosmic_text::Weight::SEMIBOLD,
        Weight::Bold => cosmic_text::Weight::BOLD,
        Weight::ExtraBold => cosmic_text::Weight::EXTRA_BOLD,
        Weight::Black => cosmic_text::Weight::BLACK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measurement runs against a real system font: a longer pattern is
    /// never narrower than a shorter one made of the same shape, and the
    /// result is stable across calls (cache returns the same value).
    #[test]
    fn widest_orders_patterns_by_length() {
        let font = Font::with_name("Noto Sans");
        let samples: Vec<String> =
            ["1023.9 MB/s", "1.0 MB/s"].iter().map(|s| s.to_string()).collect();
        let wide = widest(&font, 12, &samples);
        let one = widest(&font, 12, &samples[1..]);
        assert!(wide > one, "wide {wide} should exceed {one}");
        assert_eq!(wide, widest(&font, 12, &samples));
        // A different size invalidates the cache and measures again.
        assert!(widest(&font, 24, &samples) > wide);
    }
}
