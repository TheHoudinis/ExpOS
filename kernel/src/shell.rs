use crate::{input::Input, port, print, println, slog, vga};
use hexa_core::{
    Authority, BootReport, CapabilityBroker, Fin, Form, FormHandle, FormKind, Lifecycle,
    Operations, PimpScope, PimpSpec, Text,
};

const MAX_LINE: usize = 128;
const MAX_FORMS: usize = 12;
const MAX_HANDLES: usize = 16;

pub fn run(report: BootReport) -> ! {
    let mut shell = Shell::new(report);
    let mut input = Input::new();
    let mut line = [0_u8; MAX_LINE];
    let mut length = 0;

    println!();
    println!("Hexa command environment ready. Type 'help'.");
    slog!("HEXA_SHELL_READY\r\n");
    prompt();

    loop {
        let Some(byte) = input.poll() else {
            core::hint::spin_loop();
            continue;
        };
        match byte {
            b'\n' => {
                println!();
                if let Ok(command) = core::str::from_utf8(&line[..length]) {
                    shell.execute(command.trim());
                }
                length = 0;
                prompt();
            }
            0x08 => {
                if length > 0 {
                    length -= 1;
                    print!("\x08 \x08");
                }
            }
            printable @ 0x20..=0x7E if length < MAX_LINE - 1 => {
                line[length] = printable;
                length += 1;
                print!("{}", printable as char);
            }
            _ => {}
        }
    }
}

struct Shell {
    report: BootReport,
    forms: [Option<Form>; MAX_FORMS],
    handles: [Option<FormHandle>; MAX_HANDLES],
    broker: CapabilityBroker,
    next_fin: u32,
    journal_sequence: u32,
}

impl Shell {
    fn new(report: BootReport) -> Self {
        let mut forms = [None; MAX_FORMS];
        forms[0] = Some(Form::new(report.root_fin, "Root", FormKind::Root));
        let mut broker = CapabilityBroker::new();
        let boot_handle = broker
            .issue(
                Authority::Operator,
                report.root_fin,
                report.stable_fin,
                Operations::READ.union(Operations::EXECUTE),
                u64::MAX,
            )
            .expect("the trusted boot Handle must be issuable");
        let mut handles = [None; MAX_HANDLES];
        handles[0] = Some(boot_handle);
        Self {
            report,
            forms,
            handles,
            broker,
            next_fin: 2,
            journal_sequence: report.journal_sequence,
        }
    }

    fn execute(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or("");
        let recognized = match command {
            "help" => {
                self.help();
                true
            }
            "about" | "version" => {
                println!(
                    "HexaOS v{} interactive architecture alpha",
                    env!("CARGO_PKG_VERSION")
                );
                println!("Form-native x86_64 kernel; authority=Operator; Dimension=Stable");
                true
            }
            "clear" => {
                crate::clear_console();
                true
            }
            "echo" => {
                println!("{}", line.get(command.len()..).unwrap_or("").trim_start());
                true
            }
            "status" => {
                self.status();
                true
            }
            "forms" | "list" => {
                self.list_forms();
                true
            }
            "dimensions" | "dims" => {
                println!(
                    "Stable  FIN={}  security-boundary=yes",
                    self.report.stable_fin
                );
                true
            }
            "policy" => {
                println!("Root@Stable: use=service network=restricted isolation=enabled");
                true
            }
            "handles" => {
                self.list_handles();
                true
            }
            "journal" => {
                println!(
                    "HexaFS in-memory journal sequence #{}",
                    self.journal_sequence
                );
                true
            }
            "whoami" => {
                println!("Operator (full system authority)");
                true
            }
            "mkform" => {
                self.mkform(words.next(), words.next());
                true
            }
            "inspect" | "fin" => {
                self.inspect(words.next());
                true
            }
            "retire" => {
                self.set_lifecycle(words.next(), Lifecycle::Retired);
                true
            }
            "activate" => {
                self.set_lifecycle(words.next(), Lifecycle::Active);
                true
            }
            "grant" => {
                self.grant(words.next(), words.next());
                true
            }
            "revoke" => {
                self.revoke(words.next());
                true
            }
            "pimp" => {
                self.pimp(words.next(), words.next());
                true
            }
            "ayo" => {
                println!("ayo is currently a Go userspace development Form.");
                println!("From the host: ./ayo/bin/ayo --authority operator <command>");
                true
            }
            "reboot" => {
                println!("Rebooting HexaOS...");
                slog!("HEXA_COMMAND_OK reboot\r\n");
                port::reboot();
            }
            "shutdown" | "halt" => {
                println!("Shutting down HexaOS...");
                slog!("HEXA_COMMAND_OK shutdown\r\n");
                port::shutdown();
            }
            _ => false,
        };
        if recognized {
            slog!("HEXA_COMMAND_OK {}\r\n", command);
        } else {
            println!("Unknown command '{}'. Type 'help'.", command);
            slog!("HEXA_COMMAND_ERROR {}\r\n", command);
        }
    }

    fn help(&self) {
        println!("HexaOS commands:");
        println!("  help clear echo about status whoami");
        println!("  forms dimensions inspect <name> journal policy handles");
        println!("  mkform <name> [service|interface|package|driver|data|policy]");
        println!("  retire <name>  activate <name>");
        println!("  grant <name> <read|execute|configure|relate|retire|package>");
        println!("  revoke <handle-id>  pimp <name> <key=value>");
        println!("  ayo reboot shutdown");
    }

    fn status(&self) {
        let active = self
            .forms
            .iter()
            .flatten()
            .filter(|form| form.lifecycle == Lifecycle::Active)
            .count();
        let handle_count = self
            .handles
            .iter()
            .flatten()
            .filter(|handle| !handle.revoked)
            .count();
        println!("architecture: x86_64 Form-native alpha");
        println!("Dimension: Stable  authority: Operator");
        println!("active Forms: {}  active Handles: {}", active, handle_count);
        println!("journal sequence: {}", self.journal_sequence);
    }

    fn list_forms(&self) {
        println!("NAME             KIND        STATE       FIN");
        for form in self.forms.iter().flatten() {
            println!(
                "{:<16} {:<11} {:<11} {}",
                form.name,
                kind_name(form.kind),
                lifecycle_name(form.lifecycle),
                form.fin
            );
        }
    }

    fn mkform(&mut self, name: Option<&str>, raw_kind: Option<&str>) {
        let Some(name) = name else {
            println!("usage: mkform <name> [kind]");
            return;
        };
        if Text::new(name).is_err() {
            println!("Form names must be 1-32 ASCII characters.");
            return;
        }
        if self.find_form(name).is_some() {
            println!("A Form named '{}' already exists in Stable.", name);
            return;
        }
        let kind = match raw_kind.unwrap_or("service") {
            "service" => FormKind::Service,
            "interface" => FormKind::Interface,
            "package" => FormKind::Package,
            "driver" => FormKind::Driver,
            "data" => FormKind::Data,
            "policy" => FormKind::Policy,
            other => {
                println!("Unknown Form kind '{}'.", other);
                return;
            }
        };
        let Some(slot) = self.forms.iter_mut().find(|slot| slot.is_none()) else {
            println!("The bootstrap Form registry is full.");
            return;
        };
        let fin = Fin::from_u128(0x464F_524D_0000_0000_0000_0000_0000_0000 | self.next_fin as u128);
        self.next_fin += 1;
        *slot = Some(Form::new(fin, name, kind));
        self.commit_action();
        println!("Created and bound '{}' to Stable.", name);
        println!("FIN={}", fin);
    }

    fn inspect(&self, identity: Option<&str>) {
        let Some(identity) = identity else {
            println!("usage: inspect <Form-name>");
            return;
        };
        let Some(form) = self.find_form(identity) else {
            println!("No visible Form named '{}' in Stable.", identity);
            return;
        };
        println!("name={} FIN={}", form.name, form.fin);
        println!(
            "kind={} revision={} state={}",
            kind_name(form.kind),
            form.revision,
            lifecycle_name(form.lifecycle)
        );
        println!("Dimension=Stable");
    }

    fn set_lifecycle(&mut self, identity: Option<&str>, lifecycle: Lifecycle) {
        let Some(identity) = identity else {
            println!("usage: retire|activate <Form-name>");
            return;
        };
        let Some(index) = self.find_form_index(identity) else {
            println!("No Form named '{}'.", identity);
            return;
        };
        if self.forms[index]
            .as_ref()
            .is_some_and(|form| form.kind == FormKind::Root)
        {
            println!("DIESE denied: the Root Form cannot be retired from its boot Dimension.");
            return;
        }
        let form = self.forms[index].as_mut().unwrap();
        form.lifecycle = lifecycle;
        form.revision += 1;
        let name = form.name;
        let revision = form.revision;
        self.commit_action();
        println!(
            "{} is now {} at revision {}.",
            name,
            lifecycle_name(lifecycle),
            revision
        );
    }

    fn grant(&mut self, identity: Option<&str>, operation: Option<&str>) {
        let (Some(identity), Some(operation)) = (identity, operation) else {
            println!("usage: grant <Form-name> <operation>");
            return;
        };
        let Some(form) = self.find_form(identity) else {
            println!("No Form named '{}'.", identity);
            return;
        };
        let target = form.fin;
        let operations = match operation {
            "read" => Operations::READ,
            "execute" => Operations::EXECUTE,
            "configure" => Operations::CONFIGURE,
            "relate" => Operations::RELATE,
            "retire" => Operations::RETIRE,
            "package" => Operations::PACKAGE,
            other => {
                println!("Unknown operation '{}'.", other);
                return;
            }
        };
        let Ok(handle) = self.broker.issue(
            Authority::Operator,
            target,
            self.report.stable_fin,
            operations,
            u64::MAX,
        ) else {
            println!("DIESE denied the Handle request.");
            return;
        };
        let Some(slot) = self.handles.iter_mut().find(|slot| slot.is_none()) else {
            println!("The bootstrap Handle table is full.");
            return;
        };
        *slot = Some(handle);
        self.commit_action();
        println!(
            "Granted Handle #{} for {} on {}.",
            handle.id, operation, identity
        );
    }

    fn revoke(&mut self, raw_id: Option<&str>) {
        let Some(id) = raw_id.and_then(|value| value.parse::<u32>().ok()) else {
            println!("usage: revoke <handle-id>");
            return;
        };
        if self.broker.revoke(id).is_err() {
            println!("Handle #{} does not exist.", id);
            return;
        }
        if let Some(handle) = self
            .handles
            .iter_mut()
            .flatten()
            .find(|handle| handle.id == id)
        {
            handle.revoked = true;
        }
        self.commit_action();
        println!("Revoked Handle #{}.", id);
    }

    fn list_handles(&self) {
        println!("ID   STATE     OPS    TARGET FIN");
        for handle in self.handles.iter().flatten() {
            println!(
                "{:<4} {:<9} 0x{:02X}   {}",
                handle.id,
                if handle.revoked { "revoked" } else { "active" },
                handle.operations.bits(),
                handle.target
            );
        }
    }

    fn pimp(&mut self, identity: Option<&str>, setting: Option<&str>) {
        let (Some(identity), Some(setting)) = (identity, setting) else {
            println!("usage: pimp <Form-name> <key=value>");
            return;
        };
        let Some(form) = self.find_form(identity) else {
            println!("No Form named '{}'.", identity);
            return;
        };
        match PimpSpec::parse(
            PimpScope::Form,
            form.fin,
            Some(self.report.stable_fin),
            setting,
        ) {
            Ok(_) => {
                self.commit_action();
                println!("PIMP accepted '{}'; DIESE validation passed.", setting);
            }
            Err(error) => println!("DIESE rejected the specification: {:?}", error),
        }
    }

    fn find_form(&self, identity: &str) -> Option<&Form> {
        self.find_form_index(identity)
            .and_then(|index| self.forms[index].as_ref())
    }

    fn find_form_index(&self, identity: &str) -> Option<usize> {
        self.forms.iter().position(|slot| {
            slot.as_ref()
                .is_some_and(|form| form.name.as_str().eq_ignore_ascii_case(identity))
        })
    }

    fn commit_action(&mut self) {
        self.journal_sequence = self.journal_sequence.wrapping_add(1).max(1);
    }
}

fn prompt() {
    let mut writer = vga::WRITER.lock();
    writer.set_color(vga::Color::LightGreen, vga::Color::Black);
    drop(writer);
    print!("operator@Stable> ");
    vga::WRITER
        .lock()
        .set_color(vga::Color::LightGray, vga::Color::Black);
}

const fn kind_name(kind: FormKind) -> &'static str {
    match kind {
        FormKind::Root => "root",
        FormKind::Service => "service",
        FormKind::Interface => "interface",
        FormKind::Package => "package",
        FormKind::Driver => "driver",
        FormKind::Data => "data",
        FormKind::Policy => "policy",
    }
}

const fn lifecycle_name(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Active => "active",
        Lifecycle::Retired => "retired",
        Lifecycle::Recoverable => "recoverable",
    }
}
