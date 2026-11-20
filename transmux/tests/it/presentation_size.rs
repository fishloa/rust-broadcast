use transmux::ir::{AspectRatioIdc, DisplayGeometry, PictureRect};
use transmux::presentation_size;

#[test]
fn presentation_size_cases() {
    let ext = |w, h| {
        Some(AspectRatioIdc::ExtendedSar {
            sar_width: w,
            sar_height: h,
        })
    };
    let g = |idc| DisplayGeometry::new(720, 576, idc);

    assert_eq!(
        presentation_size(Some(&g(ext(64, 45))), 720, 576),
        (1024, 576),
        "anamorphic 64:45"
    );
    assert_eq!(
        presentation_size(Some(&g(ext(16, 11))), 720, 576),
        (1047, 576),
        "anamorphic 16:11"
    );
    assert_eq!(
        presentation_size(Some(&g(Some(AspectRatioIdc::Square))), 720, 576),
        (720, 576),
        "square pixels"
    );

    let mut cropped = DisplayGeometry::new(1920, 1088, Some(AspectRatioIdc::Square));
    cropped.output_rect = PictureRect::new(0, 0, 1920, 1080);
    assert_eq!(
        presentation_size(Some(&cropped), 1920, 1088),
        (1920, 1080),
        "cropped"
    );

    assert_eq!(
        presentation_size(None, 720, 576),
        (720, 576),
        "display none"
    );
}

/// A rescaled width is rounded to the **nearest** whole pixel, not truncated:
/// 1 px at 3:2 is 1.5 (2), at 5:4 it is 1.25 (1).
#[test]
fn a_fractional_presentation_width_rounds_to_the_nearest_pixel() {
    let mut g = DisplayGeometry::new(
        1,
        1,
        Some(AspectRatioIdc::ExtendedSar {
            sar_width: 3,
            sar_height: 2,
        }),
    );
    g.output_rect = PictureRect::new(0, 0, 1, 1);
    assert_eq!(presentation_size(Some(&g), 1, 1), (2, 1), "1.5 rounds up");

    let mut g = DisplayGeometry::new(
        1,
        1,
        Some(AspectRatioIdc::ExtendedSar {
            sar_width: 5,
            sar_height: 4,
        }),
    );
    g.output_rect = PictureRect::new(0, 0, 1, 1);
    assert_eq!(
        presentation_size(Some(&g), 1, 1),
        (1, 1),
        "1.25 rounds down"
    );
}

/// A display with a cropped output rect but no SAR keeps the cropped width;
/// only the SAR path rescales, and an unscaled width must never come out 0.
#[test]
fn a_display_without_a_sar_keeps_the_cropped_size() {
    let mut g = DisplayGeometry::new(1920, 1088, None);
    g.output_rect = PictureRect::new(0, 0, 1920, 1080);
    assert_eq!(presentation_size(Some(&g), 1920, 1088), (1920, 1080));
}
