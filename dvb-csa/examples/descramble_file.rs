use dvb_csa::{ControlWord, descramble};
use std::{env, fs, process};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: descramble_file <input.bin> <output.bin> <cw-hex>");
        eprintln!("  cw-hex: 16 hex chars, e.g. 0102030405060708");
        process::exit(1);
    }

    let mut data = fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("read {}: {e}", args[1]);
        process::exit(1);
    });

    let cw = ControlWord::from_bytes(parse_cw(&args[3]));
    descramble(&cw, &mut data);

    fs::write(&args[2], &data).unwrap_or_else(|e| {
        eprintln!("write {}: {e}", args[2]);
        process::exit(1);
    });
    eprintln!("Descrambled {} bytes → {}", data.len(), args[2]);
}

fn parse_cw(hex: &str) -> [u8; 8] {
    assert!(hex.len() == 16, "CW must be 16 hex chars (8 bytes)");
    let mut cw = [0u8; 8];
    for i in 0..8 {
        cw[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("invalid hex in CW");
    }
    cw
}
