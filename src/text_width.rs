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

/// Marked on a label that does not fit its budget (see [`fit`]).
const ELLIPSIS: &str = "…";

/// Result of the last measurement: the sections are rebuilt on every tick,
/// so the system font scan must happen only when font/size really change.
static CACHE: Mutex<Option<CacheKey>> = Mutex::new(None);

/// The font system itself, built on first use and reused after: it used
/// to scan and keep every font on the disk (~1000 faces parsed and held
/// for the process' life) while measurements only ever shape the one
/// family the bar renders with — now [`build_system`] loads just that
/// family's files (through `fc-match`), so both the ~125 ms scan and
/// the resident face database are gone. Reuse makes each measurement
/// shaping only — cheap enough to run per hover frame and per fitting
/// candidate. Fonts installed while the bar runs are picked up at the
/// next restart (the scan never repeats); the slot remembers the family
/// it was built for and rebuilds when a config reload changes it.
static SYSTEM: Mutex<Option<(String, cosmic_text::FontSystem)>> = Mutex::new(None);

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

/// Ellipsize `jobs` — `(text, budget px)` pairs — each to its own budget: a
/// label already narrow enough comes back unchanged, an overlong one is cut
/// at a character boundary with [`ELLIPSIS`] appended, and a budget too
/// small even for the ellipsis yields an empty string. Candidates are
/// shaped inside one shared font system session: a binary search measures
/// ~log2(len) of them per cut, and building a font system per candidate
/// would mean a ~125 ms disk scan each.
pub fn fit(font: &Font, font_size: u32, jobs: &[(String, u32)]) -> Vec<String> {
    let attrs = attrs(font);
    with_system(font, |font_system| {
        jobs.iter()
            .map(|(text, budget)| {
                let budget = u16::try_from(*budget).unwrap_or(u16::MAX);
                if width_px(font_system, &attrs, font_size, text) <= budget {
                    return text.clone();
                }
                if width_px(font_system, &attrs, font_size, ELLIPSIS) > budget {
                    return String::new();
                }
                // Character boundaries: a cut must not split a character.
                let mut cuts: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
                cuts.push(text.len());
                // Invariant: `lo` fits (0 = the lone ellipsis), `hi` does not.
                let (mut lo, mut hi) = (0, cuts.len() - 1);
                while lo + 1 < hi {
                    let mid = (lo + hi) / 2;
                    let candidate = format!("{}{}", &text[..cuts[mid]], ELLIPSIS);
                    if width_px(font_system, &attrs, font_size, &candidate) <= budget {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                format!("{}{}", &text[..cuts[lo]], ELLIPSIS)
            })
            .collect()
    })
}

/// The fontconfig query for a family: the configured name, or the
/// generic keyword for iced's generic families.
fn family_query(font: &Font) -> String {
    match &font.family {
        Family::Name(name) => (*name).to_string(),
        Family::SansSerif => "sans-serif".to_string(),
        Family::Serif => "serif".to_string(),
        Family::Cursive => "cursive".to_string(),
        Family::Fantasy => "fantasy".to_string(),
        Family::Monospace => "monospace".to_string(),
    }
}

/// The font system for the configured family only: `fc-match` resolves
/// the family — regular and bold, the bar can pin either weight — to
/// their files, and the full system scan (parsing and keeping every
/// face on the disk) never happens. `sans-serif` rides along as the
/// glyph-fallback candidate cosmic-text looks for. A resolution that
/// yields no file at all falls back to the full scan, so a missing
/// font can never measure every width as zero (the M5 pin would then
/// wrap the text it is meant to fix).
fn build_system(family: &str) -> cosmic_text::FontSystem {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for query in [
        family.to_string(),
        format!("{family}:bold"),
        "sans-serif".to_string(),
    ] {
        let Ok(output) = std::process::Command::new("fc-match")
            .args(["-f", "%{file}\n", &query])
            .output()
        else {
            break;
        };
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            continue;
        }
        // Probe the file before trusting it: a path fc-match knows is
        // not a guarantee the font parses.
        let mut probe = cosmic_text::fontdb::Database::new();
        if probe.load_font_file(&path).is_ok() {
            files.push(path.into());
        }
    }
    if files.is_empty() {
        return cosmic_text::FontSystem::new();
    }
    cosmic_text::FontSystem::new_with_fonts(
        files.into_iter().map(cosmic_text::fontdb::Source::File),
    )
}

/// Run `f` on the shared font system (see [`SYSTEM`]): it buffers shape
/// results in place, so access must be exclusive. The system is built
/// for the family in play and rebuilt when a config reload points the
/// bar at a different one. Lock order is always `CACHE` → `SYSTEM`
/// ([`fit`] takes `SYSTEM` alone), so the two can't deadlock against
/// each other.
fn with_system<T>(font: &Font, f: impl FnOnce(&mut cosmic_text::FontSystem) -> T) -> T {
    let family = family_query(font);
    let mut slot = SYSTEM.lock().unwrap_or_else(PoisonError::into_inner);
    let system = slot.get_or_insert_with(|| (family.clone(), build_system(&family)));
    if system.0 != family {
        *system = (family.clone(), build_system(&family));
    }
    f(&mut system.1)
}

/// The attributes iced shapes the configured font with.
fn attrs(font: &Font) -> cosmic_text::Attrs<'_> {
    cosmic_text::Attrs {
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
    }
}

/// Width of `text` on a single unbounded line (never wrapped), the larger
/// of both shaping modes iced may use — a measured value must never be
/// narrower than what gets drawn.
fn shape(
    font_system: &mut cosmic_text::FontSystem,
    attrs: &cosmic_text::Attrs,
    font_size: u32,
    text: &str,
) -> f32 {
    let mut widest = 0.0_f32;
    for shaping in [cosmic_text::Shaping::Basic, cosmic_text::Shaping::Advanced] {
        let mut buffer = cosmic_text::Buffer::new(
            font_system,
            cosmic_text::Metrics::new(font_size as f32, font_size as f32),
        );
        buffer.set_size(font_system, Some(UNBOUNDED), Some(UNBOUNDED));
        buffer.set_wrap(font_system, cosmic_text::Wrap::None);
        let _ = buffer.set_text(font_system, text, attrs, shaping, None);
        widest = widest.max(
            buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0_f32, f32::max),
        );
    }
    widest
}

/// Width of `text` as the layout takes it (rounded up), so a fit never
/// spills past the budget by a sub-pixel.
fn width_px(
    font_system: &mut cosmic_text::FontSystem,
    attrs: &cosmic_text::Attrs,
    font_size: u32,
    text: &str,
) -> u16 {
    shape(font_system, attrs, font_size, text)
        .ceil()
        .min(u16::MAX as f32) as u16
}

/// Shape every pattern in both modes and return the maximum line width
/// (one shared font system, the widest result cached by [`widest`]).
fn measure(font: &Font, font_size: u32, patterns: &[String]) -> u16 {
    let attrs = attrs(font);
    with_system(font, |font_system| {
        patterns
            .iter()
            .map(|pattern| shape(font_system, &attrs, font_size, pattern))
            .fold(0.0_f32, f32::max)
            .ceil()
            .min(u16::MAX as f32) as u16
    })
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

    /// A short label passes through untouched; an overlong one comes back
    /// with the ellipsis and — measured with the same shaper — fits its
    /// budget exactly.
    #[test]
    fn fit_keeps_short_labels_and_cuts_long_ones_to_budget() {
        let font = Font::with_name("Noto Sans");
        let short = "Apply".to_string();
        let long = "A menu label that is far too long to ever fit a narrow row".to_string();
        let budget = 60_u16;
        let fitted = fit(&font, 12, &[(short.clone(), 500), (long.clone(), u32::from(budget))]);
        assert_eq!(fitted[0], short);
        assert!(fitted[1].ends_with(ELLIPSIS), "{}", fitted[1]);
        assert!(fitted[1].len() < long.len());
        let width = with_system(&font, |font_system| width_px(font_system, &attrs(&font), 12, &fitted[1]));
        assert!(width <= budget, "{width} > {budget}");
    }

    /// The lean system (family files only, resolved by `fc-match`)
    /// still shapes real glyphs: a resolution failure falls back to the
    /// full scan instead of measuring everything as zero.
    #[test]
    fn lean_system_shapes_the_configured_family() {
        let font = Font::with_name("Noto Sans");
        let width =
            with_system(&font, |font_system| width_px(font_system, &attrs(&font), 12, "0123456789"));
        assert!(width > 0, "digits must have a width in the configured family");
    }

    /// A budget too small even for the ellipsis drops the text entirely
    /// (the row still renders its indent and toggle glyph).
    #[test]
    fn fit_returns_empty_when_the_ellipsis_does_not_fit() {
        let font = Font::with_name("Noto Sans");
        let fitted = fit(&font, 12, &[("Something".to_string(), 0)]);
        assert_eq!(fitted, vec![String::new()]);
    }
}
