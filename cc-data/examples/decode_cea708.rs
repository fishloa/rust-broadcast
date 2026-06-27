//! Decode a CEA-708 (DTVCC) caption packet to window text.
//!
//! Run with: `cargo run -p cc-data --example decode_cea708`

use cc_data::decode::Cea708Decoder;

fn main() {
    let mut dec = Cea708Decoder::new();

    // Build a Caption Channel Packet carrying one Service Block for service 1
    // that: DefineWindow DF0 (visible, 2 rows × 16 cols), writes "HI THERE".
    let block: &[u8] = &[
        0x98, // DefineWindow DF0
        0x20, // parm1: visible=YES, priority 0
        0x00, // parm2: rp=0, anchor vertical 0
        0x00, // parm3: anchor horizontal 0
        0x01, // parm4: anchor point 0, row count = 1+1 = 2
        0x0F, // parm5: column count = 15+1 = 16
        0x00, // parm6: window/pen style 0 (auto)
        b'H', b'I', b' ', b'T', b'H', b'E', b'R', b'E',
    ];

    // Service Block header: service_number = 1, block_size = block.len().
    let mut sb: Vec<u8> = Vec::new();
    sb.push((1 << 5) | (block.len() as u8));
    sb.extend_from_slice(block);

    // CCP header: sequence_number 0, packet_size_code covers the data.
    let size_code = (sb.len().div_ceil(2) + 1) as u8;
    let mut ccp: Vec<u8> = Vec::new();
    ccp.push(size_code & 0x3F);
    ccp.extend_from_slice(&sb);

    dec.push_packet(&ccp);

    println!("Service 1 text : {:?}", dec.service_text(1));
    if let Some(w) = dec.windows(1)[0].as_ref() {
        println!("Window 0 state : {}", w.state);
        println!("Window 0 rows  : {}", w.row_count);
        println!("Window 0 cols  : {}", w.column_count);
        println!("Window 0 text  : {:?}", w.text());
    }
}
