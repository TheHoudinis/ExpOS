//! Bounded event history. Console input and passwords are never recorded here.
use crate::sync::SpinMutex;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicUsize, Ordering};

const RECORDS: usize = 128;
const BYTES: usize = 224;
#[derive(Clone, Copy)]
struct Record {
    bytes: [u8; BYTES],
    len: usize,
}
impl Record {
    const EMPTY: Self = Self {
        bytes: [0; BYTES],
        len: 0,
    };
    fn text(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}
impl Write for Record {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        for ch in value.chars() {
            if ch == '\r' || ch == '\n' {
                continue;
            }
            let mut encoded = [0; 4];
            let bytes = ch.encode_utf8(&mut encoded).as_bytes();
            if self.len + bytes.len() > BYTES {
                break;
            }
            self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
        Ok(())
    }
}
struct History {
    records: [Record; RECORDS],
    next: usize,
    count: usize,
}
static HISTORY: SpinMutex<History> = SpinMutex::new(History {
    records: [Record::EMPTY; RECORDS],
    next: 0,
    count: 0,
});
static DROPPED: AtomicUsize = AtomicUsize::new(0);

pub fn record(args: fmt::Arguments<'_>) {
    let mut entry = Record::EMPTY;
    let _ = entry.write_fmt(args);
    // Never wait on a lock interrupted on this CPU.
    let Some(mut history) = HISTORY.try_lock() else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let next = history.next;
    history.records[next] = entry;
    history.next = (next + 1) % RECORDS;
    history.count = (history.count + 1).min(RECORDS);
}

pub fn visit(filter: &str, mut output: impl FnMut(&str)) -> (usize, usize, usize) {
    let filter = filter.trim();
    let needle = match filter {
        "browser" => "BROWSER",
        "network" => "HTTP",
        "services" => "SERVICE",
        "faults" => "FAULT",
        "all" => "",
        other => other,
    };
    let (records, next, count) = {
        let history = HISTORY.lock();
        (history.records, history.next, history.count)
    };
    let mut shown = 0;
    for n in 0..count {
        let text = records[(next + RECORDS - count + n) % RECORDS].text();
        if needle.is_empty() || text.contains(needle) {
            output(text);
            shown += 1;
        }
    }
    (shown, count, DROPPED.load(Ordering::Relaxed))
}

pub fn print(filter: &str) {
    let (shown, count, dropped) = visit(filter, |text| crate::println!("{}", text));
    crate::println!(
        "{} events; latest {} retained; {} dropped while busy",
        shown,
        count,
        dropped
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_utf8_record() {
        let mut entry = Record::EMPTY;
        for _ in 0..300 {
            entry.write_str("é\r\n").unwrap();
        }
        assert_eq!(entry.len, BYTES);
        assert_eq!(entry.text().chars().count(), BYTES / 2);
    }
}
