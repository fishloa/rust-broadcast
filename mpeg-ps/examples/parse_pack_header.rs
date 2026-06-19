use dvb_common::Parse;
/// Parse one pack header from inline bytes and print the SCR + mux_rate.
///
/// ```sh
/// cargo run -p mpeg-ps --example parse_pack_header
/// ```
use mpeg_ps::PackHeader;

fn main() {
    // A minimal pack header (from the fixture pattern):
    // start_code 0x000001BA, SCR=0, mux_rate=0x03363B, reserved=0x1F, stuffing=0
    let bytes = [
        0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x36, 0x3B, 0xF8,
    ];
    let header = PackHeader::parse(&bytes).unwrap();
    println!("pack_start_code:       0x000001BA");
    println!("SCR base:              {}", header.scr.base);
    println!("SCR extension:         {}", header.scr.extension);
    println!("SCR ticks:             {}", header.scr.ticks());
    println!("SCR seconds:           {:.6}", header.scr.seconds());
    println!(
        "program_mux_rate:      {} ({} B/s)",
        header.program_mux_rate,
        header.program_mux_rate * 50
    );
    println!("pack_stuffing_length:  {}", header.stuffing_length);
    println!("reserved:              {}", header.reserved);
}
