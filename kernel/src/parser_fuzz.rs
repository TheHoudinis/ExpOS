//! Reproducible host-only parser mutation campaigns, without hardware access.
//! This complements unit tests; it is not coverage-guided/sanitizer fuzzing.

#[test]
#[ignore = "bounded parser mutation campaign; run make parser-fuzz"]
fn native_image_parser_fuzz() {
    let seed = b"EXF1\x01\0\x01\0\x01\0\0\0\x02\0\x01\0\x10\0\0\0\0\0\0\0\x0f\x0bA";
    campaign("native-image", &[seed], |bytes| {
        if let Ok(image) = expos_core::NativeImage::parse(bytes) {
            assert!(image.entry < image.code.len());
            assert!(image.data.len() + image.zero_bytes <= 3840);
            assert_eq!(24 + image.code.len() + image.data.len(), bytes.len());
            assert!(bytes.len() <= expos_core::NATIVE_IMAGE_LIMIT);
        }
    });
}

pub fn campaign(name: &str, seeds: &[&[u8]], mut check: impl FnMut(&[u8])) {
    let cases = std::env::var("LOOP_FUZZ_CASES")
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .expect("LOOP_FUZZ_CASES must be a number")
        })
        .unwrap_or(100_000);
    let initial = std::env::var("LOOP_FUZZ_SEED")
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .expect("LOOP_FUZZ_SEED must be a number")
        })
        .unwrap_or(0x4c4f_4f50_0930);
    let mut state = initial;
    let mut random = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 32) as usize
    };
    for seed in seeds {
        // Every truncation of every valid seed, including the empty input.
        for length in 0..=seed.len() {
            check(&seed[..length]);
        }
    }
    for case in 0..cases {
        let mut data = seeds[random() % seeds.len()].to_vec();
        match case % 6 {
            0 => {
                let length = random() % (data.len() + 1);
                data.truncate(length);
            }
            1 => {
                for _ in 0..(random() % 8 + 1) {
                    if !data.is_empty() {
                        let offset = random() % data.len();
                        data[offset] ^= 1 << (random() % 8);
                    }
                }
            }
            2 => {
                if !data.is_empty() {
                    let offset = random() % data.len();
                    let count = (random() % 16 + 1).min(data.len() - offset);
                    data[offset..offset + count].fill([0, 0xff, 0x80, 0x7f][random() % 4]);
                }
            }
            3 => {
                let offset = random() % (data.len() + 1);
                let dictionary: &[&[u8]] = &[
                    b"\xff\xff\xff\xff",
                    b"\0\0",
                    b"\r\n",
                    b"18446744073709551615",
                    b"ffffffffffffffff",
                    b"\xc0\x0c",
                ];
                data.splice(
                    offset..offset,
                    dictionary[random() % dictionary.len()].iter().copied(),
                );
            }
            4 => {
                data.resize(random() % 4097, 0);
                for byte in &mut data {
                    *byte = random() as u8;
                }
            }
            _ => {
                if !data.is_empty() {
                    let offset = random() % data.len();
                    data[offset] = random() as u8;
                }
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&data)));
        if result.is_err() {
            let directory =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../build/fuzz-findings");
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join(format!("{name}-{initial}-{case}.bin"));
            std::fs::write(&path, &data).unwrap();
            panic!(
                "{name}: seed={initial} case={case}; input saved to {}",
                path.display()
            );
        }
    }
    std::println!("PARSER_FUZZ name={name} seed={initial} cases={cases} passed=true");
}
