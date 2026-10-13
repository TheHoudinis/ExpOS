//! Bounded on-device compiler for the freestanding ExpOS source profile.
//!
//! This is intentionally not an upstream C, Rust, or Go toolchain.  It accepts
//! the shared integer-only application profile used by Cliptick, validates a
//! real language-shaped entry point, and emits an EXF1 x86-64 image that can
//! only log and exit through the admitted Form ABI bootstrap.

use crate::{NATIVE_IMAGE_HEADER, NATIVE_IMAGE_LIMIT};

const USER_DATA: u64 = 0x4000_0000_1000;
const ABI_MAGIC: u64 = 0x4558_504f_5341_4249;
const DATA_OFFSET: u32 = 256;
const DATA_CAPACITY: usize = 4096 - DATA_OFFSET as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeLanguage {
    C,
    Rust,
    Go,
}

impl NativeLanguage {
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.as_bytes() {
            b"c" => Some(Self::C),
            b"rs" => Some(Self::Rust),
            b"go" => Some(Self::Go),
            _ => None,
        }
    }

    const fn entry_markers(self) -> &'static [&'static str] {
        match self {
            Self::C => &["main("],
            Self::Rust => &["fn main("],
            Self::Go => &["func main("],
        }
    }

    const fn log_markers(self) -> &'static [&'static str] {
        match self {
            Self::C => &["expos_log(", "puts("],
            Self::Rust => &["expos::log(", "log("],
            Self::Go => &["expos.Log("],
        }
    }

    const fn exit_markers(self) -> &'static [&'static str] {
        match self {
            Self::C => &["expos_exit(", "return "],
            Self::Rust => &["expos::exit(", "return "],
            Self::Go => &["expos.Exit("],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileError {
    SourceTooLarge,
    NonAscii,
    MissingEntry,
    InvalidString,
    InvalidResult,
    ImageTooLarge,
}

impl CompileError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::SourceTooLarge => "source exceeds the 4096-byte Form limit",
            Self::NonAscii => "the native source profile currently requires ASCII",
            Self::MissingEntry => "missing supported main entry point",
            Self::InvalidString => "log string is malformed or uses an unsupported escape",
            Self::InvalidResult => "exit result must be an unsigned 64-bit integer",
            Self::ImageTooLarge => "compiled code and data exceed the EXF1 image limit",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledNative {
    bytes: [u8; NATIVE_IMAGE_LIMIT],
    len: usize,
}

impl CompiledNative {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

pub fn compile(language: NativeLanguage, source: &str) -> Result<CompiledNative, CompileError> {
    if source.len() > NATIVE_IMAGE_LIMIT {
        return Err(CompileError::SourceTooLarge);
    }
    if !source.is_ascii() {
        return Err(CompileError::NonAscii);
    }
    if !language
        .entry_markers()
        .iter()
        .any(|marker| source.contains(marker))
    {
        return Err(CompileError::MissingEntry);
    }

    let mut message = [0_u8; DATA_CAPACITY];
    let message_len = match find_after(source, language.log_markers()) {
        Some(arguments) => parse_string(arguments, &mut message)?,
        None => 0,
    };
    let result = match find_after(source, language.exit_markers()) {
        Some(arguments) => parse_result(arguments)?,
        None => 0,
    };

    let mut code = [0_u8; 192];
    let mut cursor = 0;
    emit(&mut code, &mut cursor, &[0x41, 0x89, 0xD4])?; // mov r12d,edx
    emit(&mut code, &mut cursor, &[0x41, 0x89, 0xCD])?; // mov r13d,ecx
    emit(&mut code, &mut cursor, &[0x48, 0xBB])?; // mov rbx,request
    emit(&mut code, &mut cursor, &USER_DATA.to_le_bytes())?;
    if message_len != 0 {
        set_request_call(&mut code, &mut cursor, 1, true)?;
        emit(&mut code, &mut cursor, &[0x48, 0xC7, 0x43, 0x18])?; // args[0] = data offset
        emit(&mut code, &mut cursor, &DATA_OFFSET.to_le_bytes())?;
        emit(&mut code, &mut cursor, &[0x48, 0xC7, 0x43, 0x20])?; // args[1] = data length
        emit(&mut code, &mut cursor, &(message_len as u32).to_le_bytes())?;
        emit_call(&mut code, &mut cursor)?;
    }
    set_request_call(&mut code, &mut cursor, 8, false)?;
    emit(&mut code, &mut cursor, &[0x48, 0xB8])?; // mov rax,result
    emit(&mut code, &mut cursor, &result.to_le_bytes())?;
    emit(&mut code, &mut cursor, &[0x48, 0x89, 0x43, 0x18])?; // args[0]=rax
    emit_call(&mut code, &mut cursor)?;
    emit(&mut code, &mut cursor, &[0x0F, 0x0B])?; // exit must not return

    let total = NATIVE_IMAGE_HEADER + cursor + message_len;
    if total > NATIVE_IMAGE_LIMIT {
        return Err(CompileError::ImageTooLarge);
    }
    let mut image = CompiledNative {
        bytes: [0; NATIVE_IMAGE_LIMIT],
        len: total,
    };
    image.bytes[..4].copy_from_slice(b"EXF1");
    image.bytes[4..6].copy_from_slice(&1_u16.to_le_bytes());
    image.bytes[6..8].copy_from_slice(&1_u16.to_le_bytes());
    image.bytes[8..10].copy_from_slice(&1_u16.to_le_bytes());
    image.bytes[12..14].copy_from_slice(&(cursor as u16).to_le_bytes());
    image.bytes[14..16].copy_from_slice(&(message_len as u16).to_le_bytes());
    image.bytes[NATIVE_IMAGE_HEADER..NATIVE_IMAGE_HEADER + cursor].copy_from_slice(&code[..cursor]);
    image.bytes[NATIVE_IMAGE_HEADER + cursor..total].copy_from_slice(&message[..message_len]);
    Ok(image)
}

fn set_request_call(
    code: &mut [u8],
    cursor: &mut usize,
    call: u16,
    log_handle: bool,
) -> Result<(), CompileError> {
    emit(code, cursor, &[0x66, 0xC7, 0x43, 0x02])?;
    emit(code, cursor, &call.to_le_bytes())?;
    // mov [rbx+20], r13d/r12d (log/execute Handle supplied by admission)
    emit(
        code,
        cursor,
        if log_handle {
            &[0x44, 0x89, 0x6B, 0x14]
        } else {
            &[0x44, 0x89, 0x63, 0x14]
        },
    )
}

fn emit_call(code: &mut [u8], cursor: &mut usize) -> Result<(), CompileError> {
    emit(code, cursor, &[0x48, 0x89, 0xDF])?; // rdi=request
    emit(code, cursor, &[0x48, 0x8D, 0x73, 0x50])?; // rsi=response
    emit(code, cursor, &[0x48, 0xB8])?;
    emit(code, cursor, &ABI_MAGIC.to_le_bytes())?;
    emit(code, cursor, &[0xCD, 0x80])
}

fn emit(output: &mut [u8], cursor: &mut usize, bytes: &[u8]) -> Result<(), CompileError> {
    let end = cursor
        .checked_add(bytes.len())
        .filter(|end| *end <= output.len())
        .ok_or(CompileError::ImageTooLarge)?;
    output[*cursor..end].copy_from_slice(bytes);
    *cursor = end;
    Ok(())
}

fn find_after<'a>(source: &'a str, markers: &[&str]) -> Option<&'a str> {
    markers
        .iter()
        .filter_map(|marker| source.find(marker).map(|at| &source[at + marker.len()..]))
        .min_by_key(|rest| source.len() - rest.len())
}

fn parse_result(arguments: &str) -> Result<u64, CompileError> {
    let digits = arguments.trim_start();
    let end = digits
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return Err(CompileError::InvalidResult);
    }
    digits[..end]
        .parse::<u64>()
        .map_err(|_| CompileError::InvalidResult)
}

fn parse_string(arguments: &str, output: &mut [u8]) -> Result<usize, CompileError> {
    let bytes = arguments.trim_start().as_bytes();
    let Some(&quote @ (b'\'' | b'"')) = bytes.first() else {
        return Err(CompileError::InvalidString);
    };
    let mut input = 1;
    let mut length = 0;
    while input < bytes.len() {
        let byte = bytes[input];
        input += 1;
        if byte == quote {
            return Ok(length);
        }
        let decoded = if byte == b'\\' {
            let escaped = *bytes.get(input).ok_or(CompileError::InvalidString)?;
            input += 1;
            match escaped {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'\\' => b'\\',
                b'\'' => b'\'',
                b'"' => b'"',
                _ => return Err(CompileError::InvalidString),
            }
        } else if byte.is_ascii_control() {
            return Err(CompileError::InvalidString);
        } else {
            byte
        };
        let target = output.get_mut(length).ok_or(CompileError::ImageTooLarge)?;
        *target = decoded;
        length += 1;
    }
    Err(CompileError::InvalidString)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NativeImage;

    #[test]
    fn compiles_c_rust_and_go_to_valid_native_images() {
        for (language, source, expected) in [
            (
                NativeLanguage::C,
                "int main(void) { expos_log(\"hello C\\n\"); return 41; }",
                (&b"hello C\n"[..], 41_u64),
            ),
            (
                NativeLanguage::Rust,
                "fn main() { expos::log(\"hello Rust\"); expos::exit(42); }",
                (&b"hello Rust"[..], 42_u64),
            ),
            (
                NativeLanguage::Go,
                "package main\nfunc main() { expos.Log(\"hello Go\"); expos.Exit(43) }",
                (&b"hello Go"[..], 43_u64),
            ),
        ] {
            let compiled = compile(language, source).unwrap();
            let image = NativeImage::parse(compiled.bytes()).unwrap();
            assert_eq!(image.data, expected.0);
            assert!(image
                .code
                .windows(8)
                .any(|window| window == expected.1.to_le_bytes()));
        }
    }

    #[test]
    fn rejects_wrong_entry_bad_literals_and_unbounded_source() {
        assert_eq!(
            compile(NativeLanguage::Go, "package main"),
            Err(CompileError::MissingEntry)
        );
        assert_eq!(
            compile(NativeLanguage::Go, "func main() { expos.Log(\"bad\\x\") }"),
            Err(CompileError::InvalidString)
        );
        let source = "x".repeat(NATIVE_IMAGE_LIMIT + 1);
        assert_eq!(
            compile(NativeLanguage::C, &source),
            Err(CompileError::SourceTooLarge)
        );
    }
}
