use crate::{hardware, print, println};

pub fn execute(command: &str, args: &str) -> bool {
    match command {
        "date" | "clock" => hardware::print_date(),
        "timers" => hardware::print_clock_info(),
        "cpuinfo" => hardware::print_cpu_info(),
        "features" | "kernelcaps" => hardware::print_kernel_features(),
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
        "sysname" | "uname" => println!("ExpOS {} x86_64 Form-native", env!("CARGO_PKG_VERSION")),
        "env" => {
            println!("SYSTEM=ExpOS/8");
            println!("ARCH=x86_64");
            println!("AUTHORITY=Operator");
            println!("DIMENSION=Stable");
            println!("SHELL=Hexa-Form-CLI");
        }
        "true" => {}
        "false" => println!("false"),
        "sleep" => sleep(args),
        _ => return false,
    }
    true
}

pub fn is_command(name: &str) -> bool {
    matches!(
        name,
        "date"
            | "clock"
            | "timers"
            | "cpuinfo"
            | "features"
            | "kernelcaps"
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
            | "sysname"
            | "uname"
            | "env"
            | "true"
            | "false"
            | "sleep"
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

fn neofetch() {
    println!("OS: ExpOS v{}", env!("CARGO_PKG_VERSION"));
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
