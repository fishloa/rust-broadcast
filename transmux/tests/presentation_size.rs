use transmux::ir::{AspectRatioIdc, DisplayGeometry, PictureRect};
use transmux::presentation_size;

#[test]
fn presentation_size_anamorphic_64_45() {
    let g = DisplayGeometry::new(
        720,
        576,
        Some(AspectRatioIdc::ExtendedSar {
            sar_width: 64,
            sar_height: 45,
        }),
    );
    assert_eq!(presentation_size(Some(&g), 720, 576), (1024, 576));
}

#[test]
fn presentation_size_anamorphic_16_11() {
    let g = DisplayGeometry::new(
        720,
        576,
        Some(AspectRatioIdc::ExtendedSar {
            sar_width: 16,
            sar_height: 11,
        }),
    );
    assert_eq!(presentation_size(Some(&g), 720, 576), (1047, 576));
}

#[test]
fn presentation_size_square_pixels() {
    let g = DisplayGeometry::new(720, 576, Some(AspectRatioIdc::Square));
    assert_eq!(presentation_size(Some(&g), 720, 576), (720, 576));
}

#[test]
fn presentation_size_cropped() {
    let mut g = DisplayGeometry::new(1920, 1088, Some(AspectRatioIdc::Square));
    g.output_rect = PictureRect::new(0, 0, 1920, 1080);
    assert_eq!(presentation_size(Some(&g), 1920, 1088), (1920, 1080));
}

#[test]
fn presentation_size_display_none() {
    assert_eq!(presentation_size(None, 720, 576), (720, 576));
}
