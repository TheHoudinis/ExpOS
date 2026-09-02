use crate::{hardware, print, println, vga};

pub fn execute(command: &str, args: &str) -> bool {
    match command {
        "date" | "clock" => hardware::print_date(),
        "cpuinfo" => hardware::print_cpu_info(),
        "lspci" => hardware::print_pci(),
        "mem" | "free" => hardware::print_memory_architecture(),
        "neofetch" | "sysinfo" => neofetch(),
        "calc" => calc(args),
        "len" => println!("{}", args.len()),
        "hex" => match parse_i64(args) {
            Some(value) => println!("{:#X}", value),
            None => println!("usage: hex <integer>"),
        },
        "reverse" | "rev" => {
            for byte in args.bytes().rev() {
                print!("{}", byte as char);
            }
            println!();
        }
        "tolower" => map_ascii(args, false),
        "toupper" => map_ascii(args, true),
        "factor" => factor(args),
        "rand" => println!("{}", hardware::random_u32()),
        "dice" => dice(args),
        "ascii" => ascii(),
        "palette" => palette(),
        "morse" => morse(args),
        "fortune" => fortune(),
        "8ball" => eight_ball(),
        "cowsay" => cowsay(args),
        "banner" => banner(args),
        "logo" => logo(),
        "sysname" | "uname" => println!("HexaOS 8.0.0-alpha.3 x86_64 Form-native"),
        "env" => {
            println!("SYSTEM=HexaOS/8");
            println!("ARCH=x86_64");
            println!("AUTHORITY=Operator");
            println!("DIMENSION=Stable");
            println!("SHELL=Hexa-Form-CLI");
        }
        "true" => {}
        "false" => println!("false"),
        "sleep" => sleep(args),
        "beep" => print!("\x07"),
        "matrix" | "cmatrix" => matrix_sample(),
        "russian" => println!("Not today. The chamber is empty."),
        "insult" => println!("Your segmentation fault has better boundaries than that idea."),
        "excuse" => println!("Cosmic rays flipped the wrong FIN."),
        "compliment" => println!("Your Forms are exceptionally well identified."),
        "hack" => println!("Accessing mainframe... just kidding. Capability denied."),
        _ => return false,
    }
    true
}

pub fn is_command(name: &str) -> bool {
    matches!(
        name,
        "date"
            | "clock"
            | "cpuinfo"
            | "lspci"
            | "mem"
            | "free"
            | "neofetch"
            | "sysinfo"
            | "calc"
            | "len"
            | "hex"
            | "reverse"
            | "rev"
            | "tolower"
            | "toupper"
            | "factor"
            | "rand"
            | "dice"
            | "ascii"
            | "palette"
            | "morse"
            | "fortune"
            | "8ball"
            | "cowsay"
            | "banner"
            | "logo"
            | "sysname"
            | "uname"
            | "env"
            | "true"
            | "false"
            | "sleep"
            | "beep"
            | "matrix"
            | "cmatrix"
            | "russian"
            | "insult"
            | "excuse"
            | "compliment"
            | "hack"
    )
}

fn calc(args: &str) {
    let mut words = args.split_whitespace();
    let left = words.next().and_then(parse_i64);
    let operator = words.next();
    let right = words.next().and_then(parse_i64);
    let (Some(left), Some(operator), Some(right)) = (left, operator, right) else {
        println!("usage: calc <integer> <+|-|*|/|%> <integer>");
        return;
    };
    let result = match operator {
        "+" => left.checked_add(right),
        "-" => left.checked_sub(right),
        "*" => left.checked_mul(right),
        "/" if right != 0 => left.checked_div(right),
        "%" if right != 0 => left.checked_rem(right),
        _ => None,
    };
    match result {
        Some(value) => println!("{}", value),
        None => println!("calculation rejected: invalid operation, division by zero, or overflow"),
    }
}

fn parse_i64(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok()
}

fn map_ascii(args: &str, upper: bool) {
    for byte in args.bytes() {
        let mapped = if upper {
            byte.to_ascii_uppercase()
        } else {
            byte.to_ascii_lowercase()
        };
        print!("{}", mapped as char);
    }
    println!();
}

fn factor(args: &str) {
    let Some(mut value) = args.trim().parse::<u64>().ok().filter(|value| *value >= 2) else {
        println!("usage: factor <integer >= 2>");
        return;
    };
    print!("{}:", value);
    let mut divisor = 2;
    while divisor <= value / divisor {
        while value % divisor == 0 {
            print!(" {}", divisor);
            value /= divisor;
        }
        divisor += if divisor == 2 { 1 } else { 2 };
    }
    if value > 1 {
        print!(" {}", value);
    }
    println!();
}

fn dice(args: &str) {
    let sides = args
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|sides| (2..=1_000_000).contains(sides))
        .unwrap_or(6);
    println!("d{} => {}", sides, hardware::random_u32() % sides + 1);
}

fn ascii() {
    for byte in 32_u8..=126 {
        print!("{} ", byte as char);
        if (byte - 31) % 16 == 0 {
            println!();
        }
    }
    println!();
}

fn palette() {
    for index in 0_u8..16 {
        vga::WRITER
            .lock()
            .set_color(color(index), vga::Color::Black);
        print!("{:02} ", index);
        if index == 7 {
            println!();
        }
    }
    vga::WRITER
        .lock()
        .set_color(vga::Color::LightGray, vga::Color::Black);
    println!();
}

fn morse(args: &str) {
    for byte in args.bytes() {
        let code = match byte.to_ascii_uppercase() {
            b'A' => ".-",
            b'B' => "-...",
            b'C' => "-.-.",
            b'D' => "-..",
            b'E' => ".",
            b'F' => "..-.",
            b'G' => "--.",
            b'H' => "....",
            b'I' => "..",
            b'J' => ".---",
            b'K' => "-.-",
            b'L' => ".-..",
            b'M' => "--",
            b'N' => "-.",
            b'O' => "---",
            b'P' => ".--.",
            b'Q' => "--.-",
            b'R' => ".-.",
            b'S' => "...",
            b'T' => "-",
            b'U' => "..-",
            b'V' => "...-",
            b'W' => ".--",
            b'X' => "-..-",
            b'Y' => "-.--",
            b'Z' => "--..",
            b'0' => "-----",
            b'1' => ".----",
            b'2' => "..---",
            b'3' => "...--",
            b'4' => "....-",
            b'5' => ".....",
            b'6' => "-....",
            b'7' => "--...",
            b'8' => "---..",
            b'9' => "----.",
            b' ' => "/",
            _ => "?",
        };
        print!("{} ", code);
    }
    println!();
}

fn fortune() {
    const MESSAGES: [&str; 4] = [
        "A stable FIN outlives a fashionable name.",
        "The Dimension you test in is not always the one you ship.",
        "Narrow Handles make peaceful systems.",
        "DIESE sees a conflict in your future—and explains it.",
    ];
    println!(
        "{}",
        MESSAGES[hardware::random_u32() as usize % MESSAGES.len()]
    );
}

fn eight_ball() {
    const ANSWERS: [&str; 6] = [
        "Yes.",
        "No.",
        "Probably.",
        "Ask DIESE.",
        "PIMP says unsupported.",
        "The FINs align.",
    ];
    println!(
        "{}",
        ANSWERS[hardware::random_u32() as usize % ANSWERS.len()]
    );
}

fn cowsay(args: &str) {
    let message = if args.is_empty() { "moo" } else { args };
    println!("< {} >", message);
    println!("  \\   ^__^");
    println!("   \\  (oo)\\_______");
    println!("      (__)\\       )\\/\\");
    println!("          ||----w |");
    println!("          ||     ||");
}

fn banner(args: &str) {
    println!("========================================");
    println!(" {}", if args.is_empty() { "HEXA OS" } else { args });
    println!("========================================");
}

fn logo() {
    println!("  /\\  /\\  HEXA OS");
    println!(" /  \\/  \\ Form-native v8");
    println!(" \\  /\\  / FIN + Dimension + PIMP/DIESE");
    println!("  \\/  \\/");
}

fn neofetch() {
    logo();
    println!("OS: HexaOS v8.0.0-alpha.3");
    println!("Kernel: x86_64 Rust no_std");
    println!("Model: Form-native / Dimension-oriented");
    println!("Authority: Operator");
    hardware::print_cpu_info();
}

fn sleep(args: &str) {
    let units = args
        .trim()
        .parse::<u32>()
        .ok()
        .map(|value| value.min(10))
        .unwrap_or(1);
    for _ in 0..units {
        for _ in 0..5_000_000 {
            core::hint::spin_loop();
        }
    }
}

fn matrix_sample() {
    for row in 0..8 {
        for col in 0..40 {
            let value = 33 + (hardware::random_u32().wrapping_add(row * 17 + col) % 94) as u8;
            print!("{}", value as char);
        }
        println!();
    }
}

const fn color(index: u8) -> vga::Color {
    match index {
        0 => vga::Color::Black,
        1 => vga::Color::Blue,
        2 => vga::Color::Green,
        3 => vga::Color::Cyan,
        4 => vga::Color::Red,
        5 => vga::Color::Magenta,
        6 => vga::Color::Brown,
        7 => vga::Color::LightGray,
        8 => vga::Color::DarkGray,
        9 => vga::Color::LightBlue,
        10 => vga::Color::LightGreen,
        11 => vga::Color::LightCyan,
        12 => vga::Color::LightRed,
        13 => vga::Color::Pink,
        14 => vga::Color::Yellow,
        _ => vga::Color::White,
    }
}
