use std::{env, fs, path::PathBuf};

fn main() {
    let output_directory = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    for (source, output_name) in [
        ("trust/global_sign_root_r1.pem", "global_sign_root_r1.der"),
        (
            "trust/digicert_global_root_g2.pem",
            "digicert_global_root_g2.der",
        ),
    ] {
        println!("cargo:rerun-if-changed={source}");
        let pem = fs::read_to_string(source).expect("read HTTPS trust anchor");
        let mut encoded = String::new();
        for line in pem.lines() {
            if !line.starts_with("-----") {
                encoded.push_str(line.trim());
            }
        }
        let der = decode_base64(encoded.as_bytes()).expect("decode HTTPS trust anchor");
        fs::write(output_directory.join(output_name), der).expect("write HTTPS trust anchor DER");
    }
}

fn decode_base64(input: &[u8]) -> Result<Vec<u8>, &'static str> {
    if input.is_empty() || !input.len().is_multiple_of(4) {
        return Err("invalid base64 length");
    }
    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    for block in input.chunks(4) {
        let a = value(block[0])? as u32;
        let b = value(block[1])? as u32;
        let c = if block[2] == b'=' {
            0
        } else {
            value(block[2])? as u32
        };
        let d = if block[3] == b'=' {
            0
        } else {
            value(block[3])? as u32
        };
        let bits = (a << 18) | (b << 12) | (c << 6) | d;
        output.push((bits >> 16) as u8);
        if block[2] != b'=' {
            output.push((bits >> 8) as u8);
        }
        if block[3] != b'=' {
            output.push(bits as u8);
        }
    }
    Ok(output)
}

fn value(byte: u8) -> Result<u8, &'static str> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("invalid base64 character"),
    }
}
