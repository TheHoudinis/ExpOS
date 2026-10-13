//! Shared shell/terminal preferences, stored in the active CFC.
use crate::{expfs_store, sync::SpinMutex};
use expos_core::{Authority, CfcFin};

#[derive(Clone, Copy, Default)]
pub struct Style {
    pub prompt: u8,
    pub cursor: u8,
    pub spacing: u8,
    pub color: u8,
}
static STYLE: SpinMutex<Style> = SpinMutex::new(Style {
    prompt: 0,
    cursor: 0,
    spacing: 0,
    color: 0,
});
pub fn current() -> Style {
    *STYLE.lock()
}
pub fn load(cfc: CfcFin) {
    let mut style = Style::default();
    if let Some((bytes, len)) = expfs_store::load_data_form(cfc, "Console.state") {
        if len == 8
            && &bytes[..4] == b"CON1"
            && bytes[4..7].iter().all(|b| *b <= 2)
            && bytes[7] <= 3
        {
            style = Style {
                prompt: bytes[4],
                cursor: bytes[5],
                spacing: bytes[6],
                color: bytes[7],
            };
        }
    }
    *STYLE.lock() = style;
}
pub fn configure(cfc: CfcFin, authority: Authority, args: &str) -> &'static str {
    if authority == Authority::Guest {
        return "Sign in to change console preferences";
    }
    let mut words = args.split_whitespace();
    let key = words.next().unwrap_or("");
    let value = words.next().unwrap_or("");
    if words.next().is_some() {
        return usage();
    }
    let mut style = current();
    match (key, value) {
        ("prompt", "full") => style.prompt = 0,
        ("prompt", "compact") => style.prompt = 1,
        ("prompt", "path") => style.prompt = 2,
        ("cursor", "block") => style.cursor = 0,
        ("cursor", "beam") => style.cursor = 1,
        ("cursor", "underline") => style.cursor = 2,
        ("spacing", "tight") => style.spacing = 0,
        ("spacing", "roomy") => style.spacing = 1,
        ("spacing", "wide") => style.spacing = 2,
        ("color", "default") => style.color = 0,
        ("color", "green") => style.color = 1,
        ("color", "amber") => style.color = 2,
        ("color", "cyan") => style.color = 3,
        ("reset", "") => style = Style::default(),
        _ => return usage(),
    }
    let bytes = [
        b'C',
        b'O',
        b'N',
        b'1',
        style.prompt,
        style.cursor,
        style.spacing,
        style.color,
    ];
    *STYLE.lock() = style;
    match expfs_store::save_data_form(cfc, "Console.state", &bytes) {
        Ok(_) => "Console style saved",
        Err(_) => "Style applied for this session; preferences could not be saved",
    }
}
fn usage() -> &'static str {
    "style prompt full|compact|path; cursor block|beam|underline; spacing tight|roomy|wide; color default|green|amber|cyan; reset"
}
