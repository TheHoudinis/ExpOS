use crate::{
    input::{Input, KEY_DOWN, KEY_UP},
    port, print, println,
    session::Session,
    slog, vga,
};
use hexa_core::{
    Authority, BootReport, CapabilityBroker, Dimension, Fin, Form, FormHandle, FormKind, Lifecycle,
    Operations, PimpScope, PimpSpec, Relationship, RelationshipGraph, RelationshipKind, Text,
    GO_ABI_VERSION,
};

const MAX_LINE: usize = 128;
const MAX_FORMS: usize = 12;
const MAX_HANDLES: usize = 16;
const MAX_CONTENT: usize = 512;
const MAX_DIMENSIONS: usize = 6;
const MAX_HISTORY: usize = 8;
const AYO_FIN: Fin = Fin::from_u128(0x4159_4F00_0000_0000_0000_0000_0000_0001);
const GO_ABI_FIN: Fin = Fin::from_u128(0x474F_4142_4900_0000_0000_0000_0000_0001);

pub fn run(report: BootReport, mut input: Input, session: Session) -> ! {
    let mut shell = Shell::new(report, session);
    let mut line = [0_u8; MAX_LINE];
    let mut length = 0;
    let mut history_cursor = None;

    println!();
    println!("Hexa command environment ready. Type 'help'.");
    slog!("HEXA_SHELL_READY\r\n");
    prompt(shell.session);

    loop {
        let Some(byte) = input.poll() else {
            core::hint::spin_loop();
            continue;
        };
        match byte {
            b'\n' => {
                println!();
                if let Ok(command) = core::str::from_utf8(&line[..length]) {
                    shell.execute(command.trim(), &mut input);
                }
                length = 0;
                history_cursor = None;
                prompt(shell.session);
            }
            0x08 => {
                if length > 0 {
                    length -= 1;
                    print!("\x08 \x08");
                }
            }
            KEY_UP => {
                if shell.history_count > 0 {
                    let next = history_cursor
                        .map(|cursor: usize| (cursor + 1).min(shell.history_count - 1))
                        .unwrap_or(0);
                    replace_input_line(&mut line, &mut length, shell.recent_history(next));
                    history_cursor = Some(next);
                }
            }
            KEY_DOWN => {
                if let Some(cursor) = history_cursor {
                    if cursor == 0 {
                        replace_input_line(&mut line, &mut length, None);
                        history_cursor = None;
                    } else {
                        let next = cursor - 1;
                        replace_input_line(&mut line, &mut length, shell.recent_history(next));
                        history_cursor = Some(next);
                    }
                }
            }
            printable @ 0x20..=0x7E if length < MAX_LINE - 1 => {
                history_cursor = None;
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
    session: Session,
    forms: [Option<Form>; MAX_FORMS],
    handles: [Option<FormHandle>; MAX_HANDLES],
    broker: CapabilityBroker,
    relationships: RelationshipGraph,
    content: [FormContent; MAX_FORMS],
    dimensions: [Option<Dimension>; MAX_DIMENSIONS],
    history: [HistoryEntry; MAX_HISTORY],
    history_next: usize,
    history_count: usize,
    boot_tsc: u64,
    next_fin: u32,
    next_dimension_fin: u32,
    journal_sequence: u32,
}

impl Shell {
    fn new(report: BootReport, session: Session) -> Self {
        let mut forms = [None; MAX_FORMS];
        forms[0] = Some(Form::new(report.root_fin, "Root", FormKind::Root));
        forms[1] = Some(Form::new(AYO_FIN, "Ayo", FormKind::Package));
        forms[2] = Some(Form::new(
            crate::desktop::DISPLAY_FIN,
            "HexaDisplay",
            FormKind::Service,
        ));
        forms[3] = Some(Form::new(
            crate::desktop::BROWSER_FIN,
            "Browser",
            FormKind::Interface,
        ));
        forms[4] = Some(Form::new(GO_ABI_FIN, "GoABI", FormKind::Interface));
        let mut broker = CapabilityBroker::new();
        let boot_operations = if session.authority() == Authority::Guest {
            Operations::READ
        } else {
            Operations::READ.union(Operations::EXECUTE)
        };
        let boot_handle = broker
            .issue_for(
                report.root_fin,
                session.authority(),
                report.root_fin,
                report.stable_fin,
                boot_operations,
                u64::MAX,
            )
            .expect("the trusted boot Handle must be issuable");
        let mut handles = [None; MAX_HANDLES];
        handles[0] = Some(boot_handle);
        let mut dimensions = [None; MAX_DIMENSIONS];
        dimensions[0] = Some(Dimension::new(report.stable_fin, "Stable", true));
        let mut relationships = RelationshipGraph::new();
        relationships
            .relate(Relationship {
                source: report.root_fin,
                target: AYO_FIN,
                kind: RelationshipKind::Contains,
                dimension: Some(report.stable_fin),
            })
            .expect("the built-in ayo relationship must fit");
        relationships
            .relate(Relationship {
                source: AYO_FIN,
                target: report.root_fin,
                kind: RelationshipKind::ConfiguredBy,
                dimension: Some(report.stable_fin),
            })
            .expect("the built-in PIMP relationship must fit");
        for relationship in [
            Relationship {
                source: report.root_fin,
                target: crate::desktop::DISPLAY_FIN,
                kind: RelationshipKind::Contains,
                dimension: Some(report.stable_fin),
            },
            Relationship {
                source: crate::desktop::BROWSER_FIN,
                target: crate::desktop::DISPLAY_FIN,
                kind: RelationshipKind::DependsOn,
                dimension: Some(report.stable_fin),
            },
            Relationship {
                source: crate::desktop::DISPLAY_FIN,
                target: crate::desktop::BROWSER_FIN,
                kind: RelationshipKind::Provides,
                dimension: Some(report.stable_fin),
            },
            Relationship {
                source: AYO_FIN,
                target: GO_ABI_FIN,
                kind: RelationshipKind::DependsOn,
                dimension: Some(report.stable_fin),
            },
        ] {
            relationships
                .relate(relationship)
                .expect("built-in graphical relationship must fit");
        }
        let mut content = [FormContent::empty(); MAX_FORMS];
        let ayo_description = b"ayo v2 Package Form manager; commands: slap yeet glance chill fix ghost manifest highfive dodge vibecheck flex";
        content[1].bytes[..ayo_description.len()].copy_from_slice(ayo_description);
        content[1].length = ayo_description.len() as u16;
        seed_content(
            &mut content[2],
            b"HexaDisplay v1: owned surfaces, attach, damage, atomic commit, focus, z-order, hit testing, XRGB8888",
        );
        seed_content(
            &mut content[3],
            b"Browser Interface Form: bounded local HTML parser and native framebuffer document renderer",
        );
        seed_content(
            &mut content[4],
            b"Go ABI v1: versioned capability-gated Form, surface, browser, event, and package calls",
        );
        Self {
            report,
            session,
            forms,
            handles,
            broker,
            relationships,
            content,
            dimensions,
            history: [HistoryEntry::empty(); MAX_HISTORY],
            history_next: 0,
            history_count: 0,
            boot_tsc: crate::hardware::timestamp(),
            next_fin: 2,
            next_dimension_fin: 2,
            journal_sequence: report.journal_sequence,
        }
    }

    fn execute(&mut self, line: &str, input: &mut Input) {
        if line.is_empty() {
            return;
        }
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or("");
        let args = line.get(command.len()..).unwrap_or("").trim_start();
        if !is_sensitive_command(command) {
            self.record_history(line);
        }
        if (self.session.authority() == Authority::Guest && is_mutating_command(command))
            || (self.session.authority() == Authority::Power && is_operator_command(command))
        {
            println!(
                "DIESE denied '{}' for {} authority.",
                command,
                self.session.authority_name()
            );
            slog!("HEXA_COMMAND_DENIED {}\r\n", command);
            return;
        }
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
                println!(
                    "Form-native x86_64 kernel; authority={}; Dimension=Stable",
                    self.session.authority_name()
                );
                true
            }
            "clear" => {
                crate::clear_console();
                true
            }
            "echo" => {
                println!("{}", args);
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
            "packages" => {
                self.list_packages();
                true
            }
            "dimensions" | "dims" => {
                self.list_dimensions();
                true
            }
            "makedim" => {
                self.make_dimension(words.next());
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
            "history" => {
                self.print_history();
                true
            }
            "uptime" => {
                println!(
                    "{} TSC cycles since command environment start",
                    crate::hardware::timestamp().wrapping_sub(self.boot_tsc)
                );
                true
            }
            "ps" => {
                println!("CTX  FORM             STATE");
                println!("0    Kernel           running");
                println!("1    Root/Shell       running (bootstrap CPU)");
                true
            }
            "kstat" => {
                self.status();
                println!(
                    "registry capacity: Forms={} Handles={} Dimensions={}",
                    MAX_FORMS, MAX_HANDLES, MAX_DIMENSIONS
                );
                println!("input backends: PS/2 mouse/keyboard + COM1 polling");
                true
            }
            "ifconfig" => {
                println!("network: no v8 Driver Form bound");
                println!("legacy RTL8139 stack: make run-alpha");
                true
            }
            "netstat" => {
                println!("no active v8 network Handles or connections");
                true
            }
            "dmesg" | "bootlog" => {
                println!("[ok] x86_64 long mode, VGA, COM1");
                println!("[ok] Root Form + Stable Dimension");
                println!("[ok] PIMP/DIESE + Handle #1 + HexaFS journal #1");
                println!("[ok] Ayo Package Form + typed relationship graph");
                println!("[ok] interactive command environment");
                true
            }
            "mode" => {
                println!("active: VGA text 80x25, mirrored COM1 serial");
                println!("legacy VBE framebuffer modes: make run-alpha");
                true
            }
            "displayinfo" => {
                println!("HexaDisplay protocol v1: surfaces attach damage commit focus hit-test");
                println!(
                    "framebuffer: 800x600 XRGB8888 scanout available={}",
                    crate::framebuffer::available()
                );
                println!("buffer protocols: XRGB8888 ARGB8888 RGB565 TextCells");
                println!("primitives: clipped fill gradient alpha rounded line text");
                println!("Display FIN={}", crate::desktop::DISPLAY_FIN);
                true
            }
            "desktop" => {
                crate::desktop::run(input, false, self.session);
                true
            }
            "browser" => {
                crate::desktop::run(input, true, self.session);
                true
            }
            "goabi" => {
                println!("HexaOS Go ABI v{} FIN={}", GO_ABI_VERSION, GO_ABI_FIN);
                println!(
                    "calls: resolve authorize surface attach damage commit event navigate package"
                );
                println!("SDK: sdk/go/hexa (host emulator tested)");
                println!(
                    "native Go execution loader: not connected; scheduler/loader work remains"
                );
                true
            }
            "whoami" => {
                println!(
                    "{} ({} authority)",
                    self.session.name(),
                    self.session.authority_name()
                );
                true
            }
            "users" => {
                println!("USER             AUTHORITY  STATE");
                crate::session::visit_accounts(|name, authority| {
                    println!(
                        "{:<16} {:<10} {}",
                        name,
                        authority_name(authority),
                        if name == self.session.name() {
                            "active"
                        } else {
                            "available"
                        }
                    );
                });
                true
            }
            "useradd" => {
                let (Some(name), Some(authority), Some(password)) =
                    (words.next(), words.next(), words.next())
                else {
                    println!("usage: useradd <name> <operator|power|guest> <password>");
                    return;
                };
                let Some(authority) = parse_authority(authority) else {
                    println!("authority must be operator, power, or guest");
                    return;
                };
                match crate::session::add_account(name, password, authority) {
                    Ok(()) => {
                        println!(
                            "Created user '{}' with {} authority.",
                            name,
                            authority_name(authority)
                        );
                        slog!("HEXA_USER_CREATED {}\r\n", name);
                    }
                    Err(error) => println!("useradd: {}", error.message()),
                }
                true
            }
            "userdel" => {
                let Some(name) = words.next() else {
                    println!("usage: userdel <name>");
                    return;
                };
                match crate::session::remove_account(name, self.session.name()) {
                    Ok(()) => {
                        println!("Deleted user '{}'.", name);
                        slog!("HEXA_USER_DELETED {}\r\n", name);
                    }
                    Err(error) => println!("userdel: {}", error.message()),
                }
                true
            }
            "passwd" => {
                let (Some(name), Some(password)) = (words.next(), words.next()) else {
                    println!("usage: passwd <name> <new-password>");
                    return;
                };
                match crate::session::change_password(name, password) {
                    Ok(()) => {
                        println!("Password changed for '{}'.", name);
                        slog!("HEXA_PASSWORD_CHANGED {}\r\n", name);
                    }
                    Err(error) => println!("passwd: {}", error.message()),
                }
                true
            }
            "login" | "logout" => {
                self.session = crate::session::login(input);
                true
            }
            "mkform" => {
                self.mkform(words.next(), words.next());
                true
            }
            "view" | "cat" => {
                self.view_content(words.next(), None);
                true
            }
            "head" => {
                let identity = words.next();
                let lines = words
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(10);
                self.view_content(identity, Some(lines));
                true
            }
            "write" => {
                self.write_content(args, false);
                true
            }
            "append" => {
                self.write_content(args, true);
                true
            }
            "delete" => {
                self.set_lifecycle(words.next(), Lifecycle::Recoverable);
                true
            }
            "recover" => {
                self.set_lifecycle(words.next(), Lifecycle::Active);
                true
            }
            "move" => {
                self.move_form(words.next(), words.next());
                true
            }
            "copy" => {
                self.copy_form(words.next(), words.next());
                true
            }
            "hexdump" => {
                self.hexdump(words.next());
                true
            }
            "du" => {
                self.disk_usage(words.next());
                true
            }
            "shasum" => {
                self.hash_form(words.next());
                true
            }
            "df" => {
                println!(
                    "Forms: {}/{}  content capacity: {} bytes each",
                    self.form_count(),
                    MAX_FORMS,
                    MAX_CONTENT
                );
                true
            }
            "which" => {
                self.which(words.next());
                true
            }
            "resolve" => {
                self.resolve(words.next());
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
            "reclaim" => {
                self.reclaim(words.next());
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
            "handlecheck" => {
                self.handlecheck(words.next(), words.next(), words.next());
                true
            }
            "pimp" => {
                self.pimp(words.next(), words.next());
                true
            }
            "relate" => {
                self.relate(words.next(), words.next(), words.next());
                true
            }
            "relationships" => {
                self.list_relationships(words.next());
                true
            }
            "unrelate" => {
                self.unrelate(words.next(), words.next(), words.next());
                true
            }
            "ayo" => {
                println!("ayo v2 is registered as Package Form {}.", AYO_FIN);
                println!("Commands: slap yeet glance chill fix ghost manifest highfive dodge");
                println!("          vibecheck flex");
                println!("Host TUI: ./ayo/bin/ayo --authority operator");
                println!("CLI:      ./ayo/bin/ayo --authority operator <command>");
                true
            }
            "games" | "arcade" => {
                crate::desktop::run_games(input, self.session);
                true
            }
            "legacy" => {
                println!("HexaOS 7.2 Diamond II remains available with its full legacy stack.");
                println!("Exit QEMU, then run: make run-alpha");
                println!("It includes games, networking, persistent ATA HexaFS, tasks, events,");
                println!("framebuffer modes, users, and the original 100+ command environment.");
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
            _ => crate::compat::execute(command, args),
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
        println!("  help clear echo about status whoami users login logout");
        println!("  useradd <name> <operator|power|guest> <password>");
        println!("  userdel <name>  passwd <name> <new-password>");
        println!("  forms packages dimensions makedim inspect journal policy handles history");
        println!("  mkform <name> [service|interface|package|driver|data|policy]");
        println!("  view/cat write append head delete recover move copy");
        println!("  hexdump du shasum df which resolve retire activate reclaim");
        println!("  grant <name> <read|execute|configure|relate|retire|package|display|input>");
        println!("  revoke <id>  handlecheck <id> <requester> <operation>");
        println!("  pimp <name> <key=value>");
        println!("  relate/unrelate <source> <kind> <target>  relationships <source>");
        println!("  desktop browser displayinfo goabi");
        println!("  ayo games arcade legacy reboot shutdown");
        println!(
            "  date clock cpuinfo features kernelcaps lspci neofetch sysinfo mem free env uptime ps"
        );
        println!("  kstat dmesg bootlog ifconfig netstat mode");
        println!("  calc len hex reverse tolower toupper factor rand dice ascii");
        println!("  palette morse fortune 8ball cowsay banner logo matrix sleep");
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
        println!(
            "Dimension: Stable  user: {}  authority: {}",
            self.session.name(),
            self.session.authority_name()
        );
        println!("active Forms: {}  active Handles: {}", active, handle_count);
        println!("typed relationships: {}", self.relationships.count());
        println!("journal sequence: {}", self.journal_sequence);
    }

    fn list_dimensions(&self) {
        println!("NAME             SECURITY  FIN");
        for dimension in self.dimensions.iter().flatten() {
            println!(
                "{:<16} {:<9} {}",
                dimension.name,
                if dimension.security_boundary {
                    "explicit"
                } else {
                    "context"
                },
                dimension.fin
            );
        }
    }

    fn make_dimension(&mut self, name: Option<&str>) {
        let Some(name) = name else {
            println!("usage: makedim <name>");
            return;
        };
        if Text::new(name).is_err()
            || self
                .dimensions
                .iter()
                .flatten()
                .any(|item| item.name.as_str().eq_ignore_ascii_case(name))
        {
            println!("Dimension name is invalid or already used.");
            return;
        }
        let Some(slot) = self.dimensions.iter_mut().find(|slot| slot.is_none()) else {
            println!("Dimension registry is full.");
            return;
        };
        let fin = Fin::from_u128(
            0x4449_4D00_0000_0000_0000_0000_0000_0000 | self.next_dimension_fin as u128,
        );
        self.next_dimension_fin += 1;
        *slot = Some(Dimension::new(fin, name, false));
        self.commit_action();
        println!("Created Dimension '{}' with FIN {}.", name, fin);
        println!("Security remains contextual until an explicit policy marks a boundary.");
    }

    fn list_forms(&self) {
        println!("NAME             KIND        STATE       SIZE FIN");
        for (index, form) in self.forms.iter().enumerate() {
            let Some(form) = form else { continue };
            println!(
                "{:<16} {:<11} {:<11} {:>4} {}",
                form.name,
                kind_name(form.kind),
                lifecycle_name(form.lifecycle),
                self.content[index].length,
                form.fin
            );
        }
    }

    fn list_packages(&self) {
        println!("PACKAGE          STATE       REV  FIN");
        for form in self.forms.iter().flatten() {
            if form.kind == FormKind::Package {
                println!(
                    "{:<16} {:<11} {:>3}  {}",
                    form.name,
                    lifecycle_name(form.lifecycle),
                    form.revision,
                    form.fin
                );
            }
        }
        println!("Package mutations are committed by the Go ayo v2 engine.");
    }

    fn relate(
        &mut self,
        source_identity: Option<&str>,
        raw_kind: Option<&str>,
        target_identity: Option<&str>,
    ) {
        let (Some(source_identity), Some(raw_kind), Some(target_identity)) =
            (source_identity, raw_kind, target_identity)
        else {
            println!(
                "usage: relate <source> <depends|provides|contains|configured-by|revises> <target>"
            );
            return;
        };
        let (Some(source), Some(target)) = (
            self.find_form(source_identity).map(|form| form.fin),
            self.find_form(target_identity).map(|form| form.fin),
        ) else {
            println!("Both relationship endpoints must be visible Forms in Stable.");
            return;
        };
        let Some(kind) = parse_relationship_kind(raw_kind) else {
            println!("Unknown relationship kind '{}'.", raw_kind);
            return;
        };
        match self.relationships.relate(Relationship {
            source,
            target,
            kind,
            dimension: Some(self.report.stable_fin),
        }) {
            Ok(()) => {
                self.commit_action();
                println!(
                    "Related '{}' --{}--> '{}' in Stable.",
                    source_identity,
                    relationship_name(kind),
                    target_identity
                );
            }
            Err(error) => println!("DIESE rejected relationship: {:?}.", error),
        }
    }

    fn list_relationships(&self, source_identity: Option<&str>) {
        let Some(source_identity) = source_identity else {
            println!("usage: relationships <source>");
            return;
        };
        let Some(source) = self.find_form(source_identity).map(|form| form.fin) else {
            println!("No Form named '{}'.", source_identity);
            return;
        };
        let kinds = [
            RelationshipKind::DependsOn,
            RelationshipKind::Provides,
            RelationshipKind::Contains,
            RelationshipKind::ConfiguredBy,
            RelationshipKind::Revises,
        ];
        let mut count = 0;
        for kind in kinds {
            for target in self
                .relationships
                .targets(source, kind, Some(self.report.stable_fin))
            {
                let name = self
                    .forms
                    .iter()
                    .flatten()
                    .find(|form| form.fin == target)
                    .map(|form| form.name.as_str())
                    .unwrap_or("unbound");
                println!(
                    "{} --{}--> {} ({})",
                    source_identity,
                    relationship_name(kind),
                    name,
                    target
                );
                count += 1;
            }
        }
        if count == 0 {
            println!("No visible relationships from '{}'.", source_identity);
        }
    }

    fn unrelate(
        &mut self,
        source_identity: Option<&str>,
        raw_kind: Option<&str>,
        target_identity: Option<&str>,
    ) {
        let (Some(source_identity), Some(raw_kind), Some(target_identity)) =
            (source_identity, raw_kind, target_identity)
        else {
            println!("usage: unrelate <source> <kind> <target>");
            return;
        };
        let (Some(source), Some(target), Some(kind)) = (
            self.find_form(source_identity).map(|form| form.fin),
            self.find_form(target_identity).map(|form| form.fin),
            parse_relationship_kind(raw_kind),
        ) else {
            println!("Relationship endpoints and kind must resolve exactly.");
            return;
        };
        let relationship = Relationship {
            source,
            target,
            kind,
            dimension: Some(self.report.stable_fin),
        };
        if self.relationships.remove(relationship) {
            self.commit_action();
            println!(
                "Removed '{}' --{}--> '{}'.",
                source_identity,
                relationship_name(kind),
                target_identity
            );
        } else {
            println!("No matching relationship exists in Stable.");
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
        let index = self.find_form_index(identity).unwrap();
        println!("content={} bytes", self.content[index].length);
    }

    fn write_content(&mut self, args: &str, append: bool) {
        let Some((identity, text)) = split_first(args) else {
            println!(
                "usage: {} <Form-name> <text>",
                if append { "append" } else { "write" }
            );
            return;
        };
        let Some(index) = self.find_form_index(identity) else {
            println!("No Form named '{}'.", identity);
            return;
        };
        if self.forms[index]
            .as_ref()
            .is_some_and(|form| form.lifecycle != Lifecycle::Active)
        {
            println!("DIESE denied: the Form is not active.");
            return;
        }
        let content = &mut self.content[index];
        let mut start = if append { content.length as usize } else { 0 };
        if append && start > 0 && start < MAX_CONTENT && content.bytes[start - 1] != b'\n' {
            content.bytes[start] = b' ';
            start += 1;
        }
        let available = MAX_CONTENT.saturating_sub(start);
        let bytes = text.as_bytes();
        let written = bytes.len().min(available);
        content.bytes[start..start + written].copy_from_slice(&bytes[..written]);
        content.length = (start + written) as u16;
        if let Some(form) = self.forms[index].as_mut() {
            form.revision += 1;
        }
        self.commit_action();
        println!(
            "{} {} bytes; revision committed.",
            if append { "Appended" } else { "Wrote" },
            written
        );
        if written != bytes.len() {
            println!("Warning: content was truncated at {} bytes.", MAX_CONTENT);
        }
    }

    fn view_content(&self, identity: Option<&str>, line_limit: Option<usize>) {
        let Some(identity) = identity else {
            println!("usage: view <Form-name>");
            return;
        };
        let Some(index) = self.find_form_index(identity) else {
            println!("No Form named '{}'.", identity);
            return;
        };
        let form = self.forms[index].as_ref().unwrap();
        if form.lifecycle == Lifecycle::Recoverable {
            println!(
                "Form '{}' is recoverable; activate or recover it first.",
                identity
            );
            return;
        }
        let content = &self.content[index];
        let mut remaining_lines = line_limit.unwrap_or(usize::MAX);
        for byte in &content.bytes[..content.length as usize] {
            if remaining_lines == 0 {
                break;
            }
            print!("{}", *byte as char);
            if *byte == b'\n' {
                remaining_lines -= 1;
            }
        }
        println!();
    }

    fn move_form(&mut self, source: Option<&str>, destination: Option<&str>) {
        let (Some(source), Some(destination)) = (source, destination) else {
            println!("usage: move <Form-name> <new-name>");
            return;
        };
        if Text::new(destination).is_err() || self.find_form(destination).is_some() {
            println!("Destination name is invalid or already used.");
            return;
        }
        let Some(index) = self.find_form_index(source) else {
            println!("No Form named '{}'.", source);
            return;
        };
        let form = self.forms[index].as_mut().unwrap();
        form.name = Text::new(destination).unwrap();
        form.revision += 1;
        let fin = form.fin;
        self.commit_action();
        println!("Renamed '{}'; FIN stayed {}.", destination, fin);
    }

    fn copy_form(&mut self, source: Option<&str>, destination: Option<&str>) {
        let (Some(source), Some(destination)) = (source, destination) else {
            println!("usage: copy <Form-name> <new-name>");
            return;
        };
        if Text::new(destination).is_err() || self.find_form(destination).is_some() {
            println!("Destination name is invalid or already used.");
            return;
        }
        let Some(source_index) = self.find_form_index(source) else {
            println!("No Form named '{}'.", source);
            return;
        };
        let Some(destination_index) = self.forms.iter().position(|slot| slot.is_none()) else {
            println!("The Form registry is full.");
            return;
        };
        let source_form = self.forms[source_index].unwrap();
        let fin = Fin::from_u128(0x464F_524D_0000_0000_0000_0000_0000_0000 | self.next_fin as u128);
        self.next_fin += 1;
        self.forms[destination_index] = Some(Form::new(fin, destination, source_form.kind));
        self.content[destination_index] = self.content[source_index];
        self.commit_action();
        println!(
            "Copied '{}' into '{}' with new FIN {}.",
            source, destination, fin
        );
    }

    fn hexdump(&self, identity: Option<&str>) {
        let Some(identity) = identity else {
            println!("usage: hexdump <Form-name>");
            return;
        };
        let Some(index) = self.find_form_index(identity) else {
            println!("No Form named '{}'.", identity);
            return;
        };
        let content = &self.content[index];
        for (offset, chunk) in content.bytes[..content.length as usize]
            .chunks(16)
            .enumerate()
        {
            print!("{:04X}: ", offset * 16);
            for byte in chunk {
                print!("{:02X} ", byte);
            }
            println!();
        }
    }

    fn disk_usage(&self, identity: Option<&str>) {
        if let Some(identity) = identity {
            if let Some(index) = self.find_form_index(identity) {
                println!(
                    "{}: {} / {} bytes",
                    identity, self.content[index].length, MAX_CONTENT
                );
            } else {
                println!("No Form named '{}'.", identity);
            }
            return;
        }
        let used: usize = self
            .content
            .iter()
            .map(|content| content.length as usize)
            .sum();
        println!(
            "HexaFS bootstrap store: {} bytes used across {} Forms",
            used,
            self.form_count()
        );
    }

    fn hash_form(&self, identity: Option<&str>) {
        let Some(identity) = identity else {
            println!("usage: shasum <Form-name>");
            return;
        };
        let Some(index) = self.find_form_index(identity) else {
            println!("No Form named '{}'.", identity);
            return;
        };
        let content = &self.content[index];
        let mut hash = 5381_u32;
        for byte in &content.bytes[..content.length as usize] {
            hash = hash.wrapping_mul(33).wrapping_add(*byte as u32);
        }
        println!("{:08X}  {}  (DJB2 development hash)", hash, identity);
    }

    fn which(&self, identity: Option<&str>) {
        let Some(identity) = identity else {
            println!("usage: which <name>");
            return;
        };
        if self.find_form(identity).is_some() {
            println!("{}: Form in Stable", identity);
        } else if crate::compat::is_command(identity) || is_shell_command(identity) {
            println!("{}: kernel command", identity);
        } else {
            println!("{}: not found", identity);
        }
    }

    fn resolve(&self, identity: Option<&str>) {
        let Some(identity) = identity else {
            println!("usage: resolve <Form-name>");
            return;
        };
        let Some(form) = self.find_form(identity) else {
            println!(
                "DIESE resolution: no visible binding for '{}' in Stable.",
                identity
            );
            return;
        };
        println!("DIESE exact resolution:");
        println!("  name={} FIN={}", form.name, form.fin);
        println!(
            "  kind={} state={} Dimension=Stable",
            kind_name(form.kind),
            lifecycle_name(form.lifecycle)
        );
        println!(
            "  relationship-participation={}",
            self.relationships.involves(form.fin)
        );
    }

    fn reclaim(&mut self, identity: Option<&str>) {
        let Some(identity) = identity else {
            println!("usage: reclaim <retired-Form-name>");
            return;
        };
        let Some(index) = self.find_form_index(identity) else {
            println!("No Form named '{}'.", identity);
            return;
        };
        let form = self.forms[index].unwrap();
        if form.kind == FormKind::Root {
            println!("DIESE denied: Root identity cannot be reclaimed.");
            return;
        }
        if form.lifecycle != Lifecycle::Retired {
            println!(
                "Reclaim requires a retired Form; '{}' is {}.",
                identity,
                lifecycle_name(form.lifecycle)
            );
            return;
        }
        if self.relationships.involves(form.fin) {
            println!(
                "DIESE denied: remove all FIN relationships before reclaiming '{}'.",
                identity
            );
            return;
        }
        if self.handles.iter().flatten().any(|handle| {
            !handle.revoked && (handle.target == form.fin || handle.requester == form.fin)
        }) {
            println!(
                "DIESE denied: revoke active Handles involving '{}' before reclamation.",
                identity
            );
            return;
        }
        self.forms[index] = None;
        self.content[index] = FormContent::empty();
        self.commit_action();
        println!(
            "Reclaimed '{}' after lifecycle and relationship checks. FIN {} is no longer bound.",
            identity, form.fin
        );
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
        let Some(operations) = parse_operation(operation) else {
            println!("Unknown operation '{}'.", operation);
            return;
        };
        let Ok(handle) = self.broker.issue_for(
            self.report.root_fin,
            self.session.authority(),
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
        println!("ID   STATE     OPS    REQUESTER FIN                        TARGET FIN");
        for handle in self.handles.iter().flatten() {
            println!(
                "{:<4} {:<9} 0x{:02X}   {}  {}",
                handle.id,
                if handle.revoked { "revoked" } else { "active" },
                handle.operations.bits(),
                handle.requester,
                handle.target
            );
        }
    }

    fn handlecheck(&self, raw_id: Option<&str>, requester: Option<&str>, operation: Option<&str>) {
        let (Some(id), Some(requester), Some(operation)) = (
            raw_id.and_then(|value| value.parse::<u32>().ok()),
            requester,
            operation,
        ) else {
            println!("usage: handlecheck <id> <requester-Form> <operation>");
            return;
        };
        let Some(requester_fin) = self.find_form(requester).map(|form| form.fin) else {
            println!("Requester Form '{}' is not visible.", requester);
            return;
        };
        let Some(handle) = self.handles.iter().flatten().find(|handle| handle.id == id) else {
            println!("Handle #{} does not exist.", id);
            return;
        };
        let Some(operation) = parse_operation(operation) else {
            println!("Unknown Handle operation.");
            return;
        };
        match self.broker.authorize_requester(
            id,
            requester_fin,
            handle.target,
            self.report.stable_fin,
            operation,
            crate::hardware::timestamp(),
        ) {
            Ok(()) => println!(
                "Handle #{} authorizes {} for requester '{}'.",
                id,
                operation_name(operation),
                requester
            ),
            Err(error) => println!("DIESE denied Handle #{}: {:?}.", id, error),
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

    fn record_history(&mut self, line: &str) {
        let bytes = line.as_bytes();
        let length = bytes.len().min(MAX_LINE);
        let entry = &mut self.history[self.history_next];
        entry.bytes[..length].copy_from_slice(&bytes[..length]);
        entry.length = length as u8;
        self.history_next = (self.history_next + 1) % MAX_HISTORY;
        self.history_count = (self.history_count + 1).min(MAX_HISTORY);
    }

    fn print_history(&self) {
        let start = if self.history_count == MAX_HISTORY {
            self.history_next
        } else {
            0
        };
        for offset in 0..self.history_count {
            let index = (start + offset) % MAX_HISTORY;
            let entry = &self.history[index];
            let text = core::str::from_utf8(&entry.bytes[..entry.length as usize]).unwrap_or("?");
            println!("{:>2}  {}", offset + 1, text);
        }
    }

    fn recent_history(&self, reverse_index: usize) -> Option<&[u8]> {
        if reverse_index >= self.history_count {
            return None;
        }
        let index = (self.history_next + MAX_HISTORY - 1 - reverse_index) % MAX_HISTORY;
        let entry = &self.history[index];
        Some(&entry.bytes[..entry.length as usize])
    }

    fn form_count(&self) -> usize {
        self.forms.iter().flatten().count()
    }
}

fn replace_input_line(line: &mut [u8; MAX_LINE], length: &mut usize, value: Option<&[u8]>) {
    while *length > 0 {
        print!("\x08 \x08");
        *length -= 1;
    }
    if let Some(value) = value {
        let count = value.len().min(MAX_LINE - 1);
        line[..count].copy_from_slice(&value[..count]);
        *length = count;
        if let Ok(text) = core::str::from_utf8(&line[..count]) {
            print!("{}", text);
        }
    }
}

#[derive(Clone, Copy)]
struct FormContent {
    bytes: [u8; MAX_CONTENT],
    length: u16,
}

#[derive(Clone, Copy)]
struct HistoryEntry {
    bytes: [u8; MAX_LINE],
    length: u8,
}

impl HistoryEntry {
    const fn empty() -> Self {
        Self {
            bytes: [0; MAX_LINE],
            length: 0,
        }
    }
}

impl FormContent {
    const fn empty() -> Self {
        Self {
            bytes: [0; MAX_CONTENT],
            length: 0,
        }
    }
}

fn seed_content(content: &mut FormContent, value: &[u8]) {
    let length = value.len().min(MAX_CONTENT);
    content.bytes[..length].copy_from_slice(&value[..length]);
    content.length = length as u16;
}

fn split_first(args: &str) -> Option<(&str, &str)> {
    let split = args.find(char::is_whitespace).unwrap_or(args.len());
    if split == 0 || split == args.len() {
        return None;
    }
    Some((&args[..split], args[split..].trim_start()))
}

fn is_shell_command(name: &str) -> bool {
    matches!(
        name,
        "help"
            | "clear"
            | "echo"
            | "about"
            | "status"
            | "whoami"
            | "users"
            | "useradd"
            | "userdel"
            | "passwd"
            | "login"
            | "logout"
            | "forms"
            | "list"
            | "packages"
            | "dimensions"
            | "dims"
            | "makedim"
            | "inspect"
            | "fin"
            | "journal"
            | "history"
            | "uptime"
            | "ps"
            | "kstat"
            | "ifconfig"
            | "netstat"
            | "dmesg"
            | "bootlog"
            | "mode"
            | "displayinfo"
            | "desktop"
            | "browser"
            | "goabi"
            | "policy"
            | "handles"
            | "mkform"
            | "view"
            | "cat"
            | "head"
            | "write"
            | "append"
            | "delete"
            | "recover"
            | "move"
            | "copy"
            | "hexdump"
            | "du"
            | "shasum"
            | "df"
            | "which"
            | "resolve"
            | "retire"
            | "activate"
            | "reclaim"
            | "grant"
            | "revoke"
            | "handlecheck"
            | "pimp"
            | "relate"
            | "relationships"
            | "unrelate"
            | "ayo"
            | "legacy"
            | "games"
            | "arcade"
            | "reboot"
            | "shutdown"
            | "halt"
    )
}

fn is_mutating_command(name: &str) -> bool {
    matches!(
        name,
        "makedim"
            | "useradd"
            | "userdel"
            | "passwd"
            | "mkform"
            | "write"
            | "append"
            | "delete"
            | "recover"
            | "move"
            | "copy"
            | "retire"
            | "activate"
            | "reclaim"
            | "grant"
            | "revoke"
            | "pimp"
            | "relate"
            | "unrelate"
            | "reboot"
            | "shutdown"
            | "halt"
    )
}

fn is_operator_command(name: &str) -> bool {
    matches!(
        name,
        "makedim"
            | "useradd"
            | "userdel"
            | "passwd"
            | "delete"
            | "retire"
            | "reclaim"
            | "grant"
            | "revoke"
            | "pimp"
            | "reboot"
            | "shutdown"
            | "halt"
    )
}

fn is_sensitive_command(name: &str) -> bool {
    matches!(name, "useradd" | "passwd")
}

const fn authority_name(authority: Authority) -> &'static str {
    match authority {
        Authority::Operator => "Operator",
        Authority::Power => "Power",
        Authority::Guest => "Guest",
    }
}

fn parse_authority(value: &str) -> Option<Authority> {
    match value {
        "operator" => Some(Authority::Operator),
        "power" | "developer" => Some(Authority::Power),
        "guest" => Some(Authority::Guest),
        _ => None,
    }
}

fn parse_operation(raw: &str) -> Option<Operations> {
    match raw {
        "read" => Some(Operations::READ),
        "execute" => Some(Operations::EXECUTE),
        "configure" => Some(Operations::CONFIGURE),
        "relate" => Some(Operations::RELATE),
        "retire" => Some(Operations::RETIRE),
        "package" => Some(Operations::PACKAGE),
        "display" => Some(Operations::DISPLAY),
        "input" => Some(Operations::INPUT),
        _ => None,
    }
}

fn operation_name(operation: Operations) -> &'static str {
    if operation == Operations::READ {
        "read"
    } else if operation == Operations::EXECUTE {
        "execute"
    } else if operation == Operations::CONFIGURE {
        "configure"
    } else if operation == Operations::RELATE {
        "relate"
    } else if operation == Operations::RETIRE {
        "retire"
    } else if operation == Operations::PACKAGE {
        "package"
    } else if operation == Operations::DISPLAY {
        "display"
    } else if operation == Operations::INPUT {
        "input"
    } else {
        "unknown"
    }
}

fn parse_relationship_kind(raw: &str) -> Option<RelationshipKind> {
    match raw {
        "depends" | "depends-on" => Some(RelationshipKind::DependsOn),
        "provides" => Some(RelationshipKind::Provides),
        "contains" => Some(RelationshipKind::Contains),
        "configured-by" => Some(RelationshipKind::ConfiguredBy),
        "revises" => Some(RelationshipKind::Revises),
        _ => None,
    }
}

const fn relationship_name(kind: RelationshipKind) -> &'static str {
    match kind {
        RelationshipKind::DependsOn => "depends-on",
        RelationshipKind::Provides => "provides",
        RelationshipKind::Contains => "contains",
        RelationshipKind::ConfiguredBy => "configured-by",
        RelationshipKind::Revises => "revises",
    }
}

fn prompt(session: Session) {
    let mut writer = vga::WRITER.lock();
    writer.set_color(vga::Color::LightGreen, vga::Color::Black);
    drop(writer);
    print!("{}@Stable> ", session.name());
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
        Lifecycle::Removed => "removed",
    }
}
