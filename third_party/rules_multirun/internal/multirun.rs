use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug, Default)]
struct Instructions {
    workspace_name: String,
    jobs: usize,
    print_command: bool,
    keep_going: bool,
    output_mode: OutputMode,
    stdin_mode: StdinMode,
    commands: Vec<CommandSpec>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OutputMode {
    #[default]
    Inherit,
    Buffer,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StdinMode {
    #[default]
    Default,
    Inherit,
}

#[derive(Clone, Debug, Default)]
struct CommandSpec {
    path: String,
    tag: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
}

#[derive(Debug)]
struct CommandResult {
    index: usize,
    success: bool,
    output: Option<String>,
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(instructions_path) = args.next() else {
        eprintln!("usage: multirun <instructions> [args...]");
        std::process::exit(2);
    };
    let extra_args = args.collect::<Vec<_>>();

    match run(&instructions_path, &extra_args) {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            eprintln!("multirun: {error}");
            std::process::exit(1);
        }
    }
}

fn run(instructions_path: &str, extra_args: &[String]) -> io::Result<bool> {
    let instructions = parse_instructions(&fs::read_to_string(instructions_path)?)?;
    let commands = instructions
        .commands
        .iter()
        .map(|command| resolve_command(&instructions.workspace_name, command, extra_args))
        .collect::<io::Result<Vec<_>>>()?;

    if instructions.jobs == 1 {
        run_serial(&instructions, &commands)
    } else {
        run_parallel(&instructions, commands)
    }
}

fn resolve_command(
    workspace_name: &str,
    command: &CommandSpec,
    extra_args: &[String],
) -> io::Result<CommandSpec> {
    let runfiles_path = command.path.strip_prefix("../").map_or_else(
        || format!("{workspace_name}/{}", command.path),
        str::to_owned,
    );

    let mut resolved = command.clone();
    resolved.path = rlocation(&runfiles_path)?;
    resolved.args.extend_from_slice(extra_args);
    Ok(resolved)
}

fn run_serial(instructions: &Instructions, commands: &[CommandSpec]) -> io::Result<bool> {
    let mut success = true;
    for command in commands {
        if instructions.print_command {
            println!("{}", command.tag);
        }

        if !run_command(command, instructions.output_mode, instructions.stdin_mode)?.success {
            if instructions.keep_going {
                success = false;
            } else {
                return Ok(false);
            }
        }
    }

    Ok(success)
}

fn run_parallel(instructions: &Instructions, commands: Vec<CommandSpec>) -> io::Result<bool> {
    let worker_count = if instructions.jobs == 0 {
        commands.len().max(1)
    } else {
        instructions.jobs.max(1)
    };
    let queue = Arc::new(Mutex::new(
        commands.into_iter().enumerate().collect::<Vec<_>>(),
    ));
    let results = Arc::new(Mutex::new(Vec::new()));
    let mut workers = Vec::new();

    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let results = Arc::clone(&results);
        let output_mode = instructions.output_mode;
        let stdin_mode = instructions.stdin_mode;
        workers.push(thread::spawn(move || -> io::Result<()> {
            loop {
                let item = queue.lock().expect("queue lock poisoned").pop();
                let Some((index, command)) = item else {
                    break;
                };
                let mut result = run_command(&command, output_mode, stdin_mode)?;
                result.index = index;
                results.lock().expect("results lock poisoned").push(result);
            }
            Ok(())
        }));
    }

    for worker in workers {
        worker
            .join()
            .map_err(|_| io::Error::other("multirun worker thread panicked"))??;
    }

    let mut results = Arc::into_inner(results)
        .expect("results still referenced")
        .into_inner()
        .expect("results lock poisoned");
    results.sort_by_key(|result| result.index);

    let mut success = true;
    for result in results {
        if instructions.print_command && instructions.output_mode == OutputMode::Buffer {
            println!("{}", instructions.commands[result.index].tag);
        }
        if let Some(output) = result.output
            && !output.is_empty()
        {
            print!("{output}");
            if !output.ends_with('\n') {
                println!();
            }
        }
        if !result.success {
            success = false;
            if !instructions.keep_going {
                break;
            }
        }
    }

    Ok(success)
}

fn run_command(
    command: &CommandSpec,
    output_mode: OutputMode,
    stdin_mode: StdinMode,
) -> io::Result<CommandResult> {
    let mut process = Command::new(&command.path);
    process.args(&command.args).envs(&command.env);

    if stdin_mode == StdinMode::Inherit {
        process.stdin(Stdio::inherit());
    }

    if output_mode == OutputMode::Buffer {
        let output = process
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .output()?;
        let mut combined = String::new();
        combined.push_str(&String::from_utf8_lossy(&output.stdout));
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok(CommandResult {
            index: 0,
            success: output.status.success(),
            output: Some(combined),
        })
    } else {
        let status = process.status()?;
        Ok(CommandResult {
            index: 0,
            success: status.success(),
            output: None,
        })
    }
}

fn rlocation(path: &str) -> io::Result<String> {
    if Path::new(path).is_absolute() {
        return Ok(path.to_owned());
    }

    if let Some(runfiles_dir) = env::var_os("RUNFILES_DIR") {
        let candidate = PathBuf::from(runfiles_dir).join(path);
        if candidate.exists() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }

    if let Some(manifest) = env::var_os("RUNFILES_MANIFEST_FILE")
        && let Some(resolved) = find_in_manifest(Path::new(&manifest), path)?
    {
        return Ok(resolved);
    }

    let exe = env::current_exe()?;
    let runfiles_dir = PathBuf::from(format!("{}.runfiles", exe.display()));
    let candidate = runfiles_dir.join(path);
    if candidate.exists() {
        return Ok(candidate.to_string_lossy().into_owned());
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("runfile not found: {path}"),
    ))
}

fn find_in_manifest(manifest: &Path, path: &str) -> io::Result<Option<String>> {
    let contents = fs::read_to_string(manifest)?;
    let prefix = format!("{path} ");
    Ok(contents.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .map(std::borrow::ToOwned::to_owned)
    }))
}

fn parse_instructions(contents: &str) -> io::Result<Instructions> {
    let mut instructions = Instructions {
        jobs: 1,
        ..Instructions::default()
    };
    let mut current_command: Option<CommandSpec> = None;

    for raw_line in contents.lines() {
        if raw_line == "command" {
            if current_command.is_some() {
                return invalid_data("nested command block");
            }
            current_command = Some(CommandSpec::default());
            continue;
        }
        if raw_line == "end" {
            let Some(command) = current_command.take() else {
                return invalid_data("end without command");
            };
            instructions.commands.push(command);
            continue;
        }

        let fields = raw_line.split('\t').collect::<Vec<_>>();
        match (current_command.as_mut(), fields.as_slice()) {
            (None, ["rules_multirun", "1"]) => {}
            (None, ["workspace", value]) => instructions.workspace_name = unescape(value)?,
            (None, ["jobs", value]) => instructions.jobs = parse_usize(value)?,
            (None, ["print_command", value]) => instructions.print_command = parse_bool(value)?,
            (None, ["keep_going", value]) => instructions.keep_going = parse_bool(value)?,
            (None, ["buffer_output", value]) => {
                instructions.output_mode = if parse_bool(value)? {
                    OutputMode::Buffer
                } else {
                    OutputMode::Inherit
                };
            }
            (None, ["forward_stdin", value]) => {
                instructions.stdin_mode = if parse_bool(value)? {
                    StdinMode::Inherit
                } else {
                    StdinMode::Default
                };
            }
            (Some(command), ["tag", value]) => command.tag = unescape(value)?,
            (Some(command), ["path", value]) => command.path = unescape(value)?,
            (Some(command), ["arg", value]) => command.args.push(unescape(value)?),
            (Some(command), ["env", key, value]) => {
                command.env.insert(unescape(key)?, unescape(value)?);
            }
            _ => return invalid_data(format!("invalid instruction line: {raw_line}")),
        }
    }

    if current_command.is_some() {
        return invalid_data("unterminated command block");
    }

    Ok(instructions)
}

fn parse_bool(value: &str) -> io::Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => invalid_data(format!("invalid bool: {value}")),
    }
}

fn parse_usize(value: &str) -> io::Result<usize> {
    value
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn unescape(value: &str) -> io::Result<String> {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }

        match chars.next() {
            Some('\\') => result.push('\\'),
            Some('t') => result.push('\t'),
            Some('n') => result.push('\n'),
            Some('r') => result.push('\r'),
            Some(other) => return invalid_data(format!("invalid escape: \\{other}")),
            None => return invalid_data("unterminated escape"),
        }
    }
    Ok(result)
}

fn invalid_data<T>(message: impl Into<String>) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidData, message.into()))
}
