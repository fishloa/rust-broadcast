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
