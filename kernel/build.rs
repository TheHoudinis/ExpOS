use std::{env, fs, path::PathBuf};

fn main() {
    if env::var("TARGET").unwrap_or_default() == "x86_64-unknown-none" {
        println!("cargo:rerun-if-changed=../ports/python");
        println!("cargo:rerun-if-changed=../vendor/micropython");
        let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .parent()
            .unwrap()
            .to_path_buf();
        println!("cargo:rerun-if-changed=../ports/services");
        println!("cargo:rerun-if-changed=../sdk/c");
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let python_build = out.join("python-service");
        let status = std::process::Command::new("make")
            .args(["-C", "ports/python", "-j4"])
            .arg(format!("BUILD={}", python_build.display()))
            .arg("EXTRA_CFLAGS=-mcmodel=large")
            .current_dir(&root)
            .status()
            .expect("build isolated MicroPython");
        assert!(status.success(), "MicroPython service build failed");
        let status = std::process::Command::new("cc")
            .args([
                "-Os",
                "-ffreestanding",
                "-fno-pie",
                "-no-pie",
                "-fno-stack-protector",
                "-mno-red-zone",
                "-mgeneral-regs-only",
                "-mcmodel=large",
                "-nostdlib",
                "-static",
                "-Wl,--build-id=none",
                "-Wl,--gc-sections",
                "-Wl,-T,ports/services/service.ld",
                "-I",
                "sdk/c",
                "ports/services/python_entry.c",
            ])
            .arg(python_build.join("libexpos_python.a"))
            .arg("-o")
            .arg(out.join("python-service.elf"))
            .current_dir(&root)
            .status()
            .expect("link isolated MicroPython");
        assert!(status.success(), "MicroPython service link failed");
        println!("cargo:rerun-if-changed=../crates/expos-core/src");
        let http_build = out.join("http-service");
        let status = std::process::Command::new("cargo")
            .args([
                "build",
                "--release",
                "--manifest-path",
                "ports/services/http/Cargo.toml",
                "--target",
                "x86_64-unknown-none",
            ])
            .arg("--target-dir")
            .arg(&http_build)
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                "-Ccode-model=large\u{1f}-Crelocation-model=static",
            )
            .current_dir(&root)
            .status()
            .expect("build HTTP service");
        assert!(status.success(), "HTTP service build failed");
        let status = std::process::Command::new("ld")
            .args(["-T", "ports/services/service.ld", "--gc-sections"])
            .arg("-o")
            .arg(out.join("http-service.elf"))
            .arg(http_build.join("x86_64-unknown-none/release/libexpos_http_service.a"))
            .arg(python_build.join("libexpos_python.a"))
            .current_dir(&root)
            .status()
            .expect("link HTTP service");
        assert!(status.success(), "HTTP service link failed");
        let network_build = out.join("network-service");
        let status = std::process::Command::new("cargo")
            .args([
                "build",
                "--release",
                "--manifest-path",
                "ports/services/network/Cargo.toml",
                "--target",
                "x86_64-unknown-none",
            ])
            .arg("--target-dir")
            .arg(&network_build)
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                "-Ccode-model=large\u{1f}-Crelocation-model=static\u{1f}--cfg=polyval_force_soft\u{1f}--cfg=aes_force_soft",
            )
            .current_dir(&root)
            .status()
            .expect("build network service");
        assert!(status.success(), "network service build failed");
        let status = std::process::Command::new("ld")
            .args(["-T", "ports/services/service.ld", "--gc-sections"])
            .arg("-o")
            .arg(out.join("network-service.elf"))
            .arg(network_build.join("x86_64-unknown-none/release/libexpos_network_service.a"))
            .arg(python_build.join("libexpos_python.a"))
            .current_dir(&root)
            .status()
            .expect("link network service");
        assert!(status.success(), "network service link failed");
    }
    let output_directory = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    for (source, output_name) in [
        ("trust/global_sign_root_r1.pem", "global_sign_root_r1.der"),
        (
            "trust/digicert_global_root_g2.pem",
            "digicert_global_root_g2.der",
        ),
        ("trust/isrg_root_x1.pem", "isrg_root_x1.der"),
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
