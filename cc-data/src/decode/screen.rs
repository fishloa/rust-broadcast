//! Shared caption display model — colour / opacity / edge / font enumerations
//! and pen attributes, per the CEA-708 conformance model (47 CFR §79.102 (h)–(q),
//! ANSI/CTA-708-E §8.5, §8.8; `private/specs-verified/captions/cc-data/decode/cea708-conformance.md`,
//! `cea708-decode.md`).

/// Masks of the packed enum fields (47 CFR §79.102, ANSI/CTA-708-E §8.5/§8.8).
const OPACITY_MASK: u8 = 0x03;
const EDGE_TYPE_MASK: u8 = 0x07;
const PEN_SIZE_MASK: u8 = 0x03;
const PEN_OFFSET_MASK: u8 = 0x03;
const FONT_STYLE_MASK: u8 = 0x07;
const JUSTIFY_MASK: u8 = 0x03;
const PRINT_DIRECTION_MASK: u8 = 0x03;
const SCROLL_DIRECTION_MASK: u8 = 0x03;

/// A CEA-708 colour: 2 bits per RGB component (R, G, B each 0–3), 64 colours
/// total (§8.8, Tables 30/31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Color {
    /// Red component, 0–3.
    pub r: u8,
    /// Green component, 0–3.
    pub g: u8,
    /// Blue component, 0–3.
    pub b: u8,
}

impl Color {
    /// Black `(0,0,0)`.
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    /// White `(2,2,2)`.
    pub const WHITE: Color = Color { r: 2, g: 2, b: 2 };

    /// Construct from three 2-bit components (masked to 0–3).
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Color {
            r: r & 0x03,
            g: g & 0x03,
            b: b & 0x03,
        }
    }

    /// Map this colour onto the minimum 8-colour list (CEA-708 §8.8, Table 30):
    /// component `0 | 1 -> 0`, `2 | 3 -> 2`.
    #[must_use]
    pub const fn to_8_color(self) -> Color {
        const fn quantise(c: u8) -> u8 {
            match c {
                0 | 1 => 0,
                _ => 2,
            }
        }
        Color {
            r: quantise(self.r),
            g: quantise(self.g),
            b: quantise(self.b),
        }
    }
}

/// Opacity of a foreground / background / fill (§8.8; SPC/SWA opacity field,
/// `cea708-decode.md`). 2-bit field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Opacity {
    /// Fully opaque.
    #[default]
    Solid,
    /// Alternates opaque / transparent.
    Flash,
    /// Partially transparent (background shows through).
    Translucent,
    /// Fully transparent (not rendered).
    Transparent,
}

impl Opacity {
    /// From the 2-bit wire value.
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & OPACITY_MASK {
            0 => Self::Solid,
            1 => Self::Flash,
            2 => Self::Translucent,
            _ => Self::Transparent,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Flash => "flash",
            Self::Translucent => "translucent",
            Self::Transparent => "transparent",
        }
    }
}
broadcast_core::impl_spec_display!(Opacity);

/// Character edge type (§79.102 (p) / SPA edge-type field; 3-bit). 0–5 defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EdgeType {
    /// No edge.
    #[default]
    None,
    /// Raised edge.
    Raised,
    /// Depressed edge.
    Depressed,
    /// Uniform outline.
    Uniform,
    /// Left drop shadow.
    LeftDropShadow,
    /// Right drop shadow.
    RightDropShadow,
}

impl EdgeType {
    /// From the 3-bit wire value (values 6–7 fold to `None`).
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & EDGE_TYPE_MASK {
            0 => Self::None,
            1 => Self::Raised,
            2 => Self::Depressed,
            3 => Self::Uniform,
            4 => Self::LeftDropShadow,
            5 => Self::RightDropShadow,
            _ => Self::None,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Raised => "raised",
            Self::Depressed => "depressed",
            Self::Uniform => "uniform",
            Self::LeftDropShadow => "left_drop_shadow",
            Self::RightDropShadow => "right_drop_shadow",
        }
    }
}
broadcast_core::impl_spec_display!(EdgeType);

/// Pen size (§79.102 (j) / SPA pen-size field; 2-bit). Value 3 is reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PenSize {
    /// Small.
    Small,
    /// Standard (default).
    #[default]
    Standard,
    /// Large.
    Large,
}

impl PenSize {
    /// From the 2-bit wire value (3 folds to `Standard`).
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & PEN_SIZE_MASK {
            0 => Self::Small,
            2 => Self::Large,
            _ => Self::Standard,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Standard => "standard",
            Self::Large => "large",
        }
    }
}
broadcast_core::impl_spec_display!(PenSize);

/// Pen vertical offset (§79.102 (l) / SPA offset field; 2-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PenOffset {
    /// Subscript.
    Subscript,
    /// Normal (default).
    #[default]
    Normal,
    /// Superscript.
    Superscript,
}

impl PenOffset {
    /// From the 2-bit wire value (3 folds to `Normal`).
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & PEN_OFFSET_MASK {
            0 => Self::Subscript,
            2 => Self::Superscript,
            _ => Self::Normal,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Subscript => "subscript",
            Self::Normal => "normal",
            Self::Superscript => "superscript",
        }
    }
}
broadcast_core::impl_spec_display!(PenOffset);

/// Font style (§79.102 (k) / SPA font-style field; 3-bit, 0–7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FontStyle {
    /// Default / undefined.
    #[default]
    Default,
    /// Monospaced with serifs.
    MonospacedSerif,
    /// Proportionally spaced with serifs.
    ProportionalSerif,
    /// Monospaced without serifs.
    MonospacedSansSerif,
    /// Proportionally spaced without serifs.
    ProportionalSansSerif,
    /// Casual.
    Casual,
    /// Cursive.
    Cursive,
    /// Small capitals.
    SmallCapitals,
}

impl FontStyle {
    /// From the 3-bit wire value.
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & FONT_STYLE_MASK {
            0 => Self::Default,
            1 => Self::MonospacedSerif,
            2 => Self::ProportionalSerif,
            3 => Self::MonospacedSansSerif,
            4 => Self::ProportionalSansSerif,
            5 => Self::Casual,
            6 => Self::Cursive,
            _ => Self::SmallCapitals,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::MonospacedSerif => "monospaced_serif",
            Self::ProportionalSerif => "proportional_serif",
            Self::MonospacedSansSerif => "monospaced_sans_serif",
            Self::ProportionalSansSerif => "proportional_sans_serif",
            Self::Casual => "casual",
            Self::Cursive => "cursive",
            Self::SmallCapitals => "small_capitals",
        }
    }
}
broadcast_core::impl_spec_display!(FontStyle);

/// Text justification (SWA justify field; 2-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Justify {
    /// Left-justified (default).
    #[default]
    Left,
    /// Right-justified.
    Right,
    /// Centred.
    Center,
    /// Fully justified.
    Full,
}

impl Justify {
    /// From the 2-bit wire value.
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & JUSTIFY_MASK {
            0 => Self::Left,
            1 => Self::Right,
            2 => Self::Center,
            _ => Self::Full,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Center => "center",
            Self::Full => "full",
        }
    }
}
broadcast_core::impl_spec_display!(Justify);

/// Print direction (SWA print-direction field; 2-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PrintDirection {
    /// Left to right (default).
    #[default]
    LeftToRight,
    /// Right to left.
    RightToLeft,
    /// Top to bottom.
    TopToBottom,
    /// Bottom to top.
    BottomToTop,
}

impl PrintDirection {
    /// From the 2-bit wire value.
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & PRINT_DIRECTION_MASK {
            0 => Self::LeftToRight,
            1 => Self::RightToLeft,
            2 => Self::TopToBottom,
            _ => Self::BottomToTop,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::LeftToRight => "left_to_right",
            Self::RightToLeft => "right_to_left",
            Self::TopToBottom => "top_to_bottom",
            Self::BottomToTop => "bottom_to_top",
        }
    }
}
broadcast_core::impl_spec_display!(PrintDirection);

/// Scroll direction (SWA scroll-direction field; 2-bit). Same wire mapping as
/// [`PrintDirection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ScrollDirection {
    /// Left to right.
    LeftToRight,
    /// Right to left.
    RightToLeft,
    /// Top to bottom.
    TopToBottom,
    /// Bottom to top (default — NTSC roll-up).
    #[default]
    BottomToTop,
}

impl ScrollDirection {
    /// From the 2-bit wire value.
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & SCROLL_DIRECTION_MASK {
            0 => Self::LeftToRight,
            1 => Self::RightToLeft,
            2 => Self::TopToBottom,
            _ => Self::BottomToTop,
        }
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::LeftToRight => "left_to_right",
            Self::RightToLeft => "right_to_left",
            Self::TopToBottom => "top_to_bottom",
            Self::BottomToTop => "bottom_to_top",
        }
    }
}
broadcast_core::impl_spec_display!(ScrollDirection);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn to_8_color_follows_the_section_8_8_mapping() {
        assert_eq!(Color::new(1, 2, 3).to_8_color(), Color::new(0, 2, 2));
        assert_eq!(Color::new(3, 3, 3).to_8_color(), Color::WHITE);
        assert_eq!(Color::new(1, 1, 1).to_8_color(), Color::BLACK);
        for r in 0..4 {
            for g in 0..4 {
                for b in 0..4 {
                    let c = Color::new(r, g, b).to_8_color();
                    assert!([c.r, c.g, c.b].iter().all(|&v| v == 0 || v == 2));
                    assert_eq!(c.to_8_color(), c);
                }
            }
        }
    }

    #[test]
    fn opacity_bits() {
        assert_eq!(Opacity::from_bits(0), Opacity::Solid);
        assert_eq!(Opacity::from_bits(3), Opacity::Transparent);
        assert_eq!(Opacity::Translucent.to_string(), "translucent");
    }

    #[test]
    fn edge_bits() {
        assert_eq!(EdgeType::from_bits(5), EdgeType::RightDropShadow);
        assert_eq!(EdgeType::from_bits(7), EdgeType::None);
    }

    #[test]
    fn font_bits() {
        assert_eq!(FontStyle::from_bits(7), FontStyle::SmallCapitals);
        assert_eq!(FontStyle::from_bits(1).to_string(), "monospaced_serif");
    }

    #[test]
    fn opacity_table_and_labels() {
        let expected = [
            Opacity::Solid,
            Opacity::Flash,
            Opacity::Translucent,
            Opacity::Transparent,
        ];
        let names = ["solid", "flash", "translucent", "transparent"];
        for v in 0..=3u8 {
            assert_eq!(Opacity::from_bits(v), expected[v as usize]);
            assert_eq!(Opacity::from_bits(v).name(), names[v as usize]);
        }
        assert_eq!(Opacity::from_bits(0x06), Opacity::Translucent);
    }

    #[test]
    fn edge_type_table_and_labels() {
        let expected = [
            EdgeType::None,
            EdgeType::Raised,
            EdgeType::Depressed,
            EdgeType::Uniform,
            EdgeType::LeftDropShadow,
            EdgeType::RightDropShadow,
            EdgeType::None,
            EdgeType::None,
        ];
        let names = [
            "none",
            "raised",
            "depressed",
            "uniform",
            "left_drop_shadow",
            "right_drop_shadow",
            "none",
            "none",
        ];
        for v in 0..=7u8 {
            assert_eq!(EdgeType::from_bits(v), expected[v as usize], "v={v}");
            assert_eq!(EdgeType::from_bits(v).name(), names[v as usize], "v={v}");
        }
    }

    #[test]
    fn pen_size_table_and_labels() {
        let expected = [
            PenSize::Small,
            PenSize::Standard,
            PenSize::Large,
            PenSize::Standard,
        ];
        let names = ["small", "standard", "large", "standard"];
        for v in 0..=3u8 {
            assert_eq!(PenSize::from_bits(v), expected[v as usize]);
            assert_eq!(PenSize::from_bits(v).name(), names[v as usize]);
        }
    }

    #[test]
    fn pen_offset_table_and_labels() {
        let expected = [
            PenOffset::Subscript,
            PenOffset::Normal,
            PenOffset::Superscript,
            PenOffset::Normal,
        ];
        let names = ["subscript", "normal", "superscript", "normal"];
        for v in 0..=3u8 {
            assert_eq!(PenOffset::from_bits(v), expected[v as usize]);
            assert_eq!(PenOffset::from_bits(v).name(), names[v as usize]);
        }
    }

    #[test]
    fn font_style_table_and_labels() {
        let expected = [
            FontStyle::Default,
            FontStyle::MonospacedSerif,
            FontStyle::ProportionalSerif,
            FontStyle::MonospacedSansSerif,
            FontStyle::ProportionalSansSerif,
            FontStyle::Casual,
            FontStyle::Cursive,
            FontStyle::SmallCapitals,
        ];
        let names = [
            "default",
            "monospaced_serif",
            "proportional_serif",
            "monospaced_sans_serif",
            "proportional_sans_serif",
            "casual",
            "cursive",
            "small_capitals",
        ];
        for v in 0..=7u8 {
            assert_eq!(FontStyle::from_bits(v), expected[v as usize], "v={v}");
            assert_eq!(FontStyle::from_bits(v).name(), names[v as usize], "v={v}");
        }
    }

    #[test]
    fn justify_table_and_labels() {
        let expected = [
            Justify::Left,
            Justify::Right,
            Justify::Center,
            Justify::Full,
        ];
        let names = ["left", "right", "center", "full"];
        for v in 0..=3u8 {
            assert_eq!(Justify::from_bits(v), expected[v as usize]);
            assert_eq!(Justify::from_bits(v).name(), names[v as usize]);
        }
    }

    #[test]
    fn print_direction_table_and_labels() {
        let expected = [
            PrintDirection::LeftToRight,
            PrintDirection::RightToLeft,
            PrintDirection::TopToBottom,
            PrintDirection::BottomToTop,
        ];
        let names = [
            "left_to_right",
            "right_to_left",
            "top_to_bottom",
            "bottom_to_top",
        ];
        for v in 0..=3u8 {
            assert_eq!(PrintDirection::from_bits(v), expected[v as usize]);
            assert_eq!(PrintDirection::from_bits(v).name(), names[v as usize]);
        }
    }

    #[test]
    fn scroll_direction_table_and_labels() {
        let expected = [
            ScrollDirection::LeftToRight,
            ScrollDirection::RightToLeft,
            ScrollDirection::TopToBottom,
            ScrollDirection::BottomToTop,
        ];
        let names = [
            "left_to_right",
            "right_to_left",
            "top_to_bottom",
            "bottom_to_top",
        ];
        for v in 0..=3u8 {
            assert_eq!(ScrollDirection::from_bits(v), expected[v as usize]);
            assert_eq!(ScrollDirection::from_bits(v).name(), names[v as usize]);
        }
    }

    #[test]
    fn from_bits_masks_to_its_field_width() {
        // High bits outside the 2-/3-bit field must be ignored.
        assert_eq!(Opacity::from_bits(0b0000_0101), Opacity::Flash);
        assert_eq!(EdgeType::from_bits(0b0000_1010), EdgeType::Depressed);
        assert_eq!(PenSize::from_bits(0b0000_0110), PenSize::Large);
        assert_eq!(PenOffset::from_bits(0b0000_0101), PenOffset::Normal);
        assert_eq!(PenOffset::from_bits(0b0000_0100), PenOffset::Subscript);
        assert_eq!(
            FontStyle::from_bits(0b0000_1010),
            FontStyle::ProportionalSerif
        );
        assert_eq!(Justify::from_bits(0b0000_0110), Justify::Center);
        assert_eq!(
            PrintDirection::from_bits(0b0000_0101),
            PrintDirection::RightToLeft
        );
        assert_eq!(
            ScrollDirection::from_bits(0b0000_0110),
            ScrollDirection::TopToBottom
        );
    }
}
