const MAJOR_SHIFT: u32 = 16;
const MINOR_SHIFT: u32 = 8;
const COMPONENT_MASK: u32 = 0xff;

fn main() {
    let version = ffmpeg_next::codec::version();
    println!(
        "libavcodec {}.{}.{}",
        version >> MAJOR_SHIFT,
        (version >> MINOR_SHIFT) & COMPONENT_MASK,
        version & COMPONENT_MASK
    );
}
