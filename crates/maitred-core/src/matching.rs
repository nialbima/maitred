//! Matching a job's argv against its project's class file (ARCHITECTURE, Class files, Matching).
//! No shell parsing on this path: the argv is what the operating system handed over. Pure: the
//! caller supplies the working directory, the project root and the machine config's lists.

use crate::classes::{ClassFile, Rule};

/// What the machine config adds to the built-in lists.
#[derive(Debug, Clone, Default)]
pub struct Machine {
    /// Wrappers recognised by name, with no options of their own.
    pub wrappers: Vec<String>,
    /// Launchers, each a sequence of words: `bundle exec`, `npx`, `ruby -S`.
    pub launchers: Vec<Vec<String>>,
    /// Programs that keep the machine awake, beside `caffeinate` and `systemd-inhibit`.
    pub keep_awake: Vec<String>,
    /// Commands that print the project's root, beside `git rev-parse --show-toplevel`: the spotter
    /// reads `$(repo-root)/bin/x` as `bin/x` from the root.
    pub root_commands: Vec<String>,
}

/// The job as it was handed over.
#[derive(Debug, Clone)]
pub struct Job {
    pub argv: Vec<String>,
    pub cwd: String,
    pub root: String,
}

/// What a job's argv is, for its class.
#[derive(Debug, Clone)]
pub enum Classified<'a> {
    /// The first rule that applies, and the command its pattern saw.
    Rule { rule: &'a Rule, command: String },
    /// No rule applies: the job is heavy.
    NoRule { command: String },
    /// A keep-awake wrapper is the program: the job is refused.
    KeepAwake { program: String },
    /// A shell running a string: the spotter finds its programs.
    Shell { shell: String, script: String },
}

const KEEP_AWAKE: [&str; 2] = ["caffeinate", "systemd-inhibit"];
pub(crate) const SHELLS: [&str; 3] = ["sh", "bash", "zsh"];

/// Classifies a job's argv: steps 1 to 7 of Matching.
#[must_use]
pub fn classify<'a>(file: &'a ClassFile, job: &Job, machine: &Machine) -> Classified<'a> {
    // 1. Wrappers, repeatedly; 2. the program normalised; 3. launchers, then normalised again.
    let (mut words, dir) = strip_wrappers(job.argv.clone(), &machine.wrappers, &job.cwd);
    normalise_program(&mut words, &dir, &job.root);
    if let Some(launcher) = machine
        .launchers
        .iter()
        .find(|l| !l.is_empty() && words.starts_with(l))
    {
        words.drain(..launcher.len());
        normalise_program(&mut words, &dir, &job.root);
    }
    let Some(program) = words.first() else {
        return Classified::NoRule {
            command: String::new(),
        };
    };
    let name = basename(program);
    // 6. A keep-awake wrapper as the program refuses the job.
    if KEEP_AWAKE.contains(&name) || machine.keep_awake.iter().any(|k| k == name) {
        return Classified::KeepAwake {
            program: name.to_owned(),
        };
    }
    // 7. A shell running a string hands it to the spotter.
    if SHELLS.contains(&name)
        && let Some(script) = shell_script(&words[1..])
    {
        return Classified::Shell {
            shell: name.to_owned(),
            script,
        };
    }
    // 4. Join; 5. the first rule that applies wins. A rule applies only to its own program, so no
    // pattern, however it is written, classes another one.
    let command = join(&words);
    match file
        .rules
        .iter()
        .find(|r| &r.program == program && r.pattern.is_match(&command))
    {
        Some(rule) => Classified::Rule { rule, command },
        None => Classified::NoRule { command },
    }
}

/// Strips leading assignments and wrappers, with their options, until neither is first. Also gives
/// the directory the rest runs in: `cwd`, moved by any `env -C` on the way.
pub(crate) fn strip_wrappers(
    mut words: Vec<String>,
    extra: &[String],
    cwd: &str,
) -> (Vec<String>, String) {
    let mut dir = lexical(cwd);
    loop {
        let assignments = words.iter().take_while(|w| is_assignment(w)).count();
        words.drain(..assignments);
        let Some(first) = words.first() else {
            return (words, dir);
        };
        let name = basename(first);
        let grammar = match grammar(name) {
            Some(grammar) => grammar,
            None if extra.iter().any(|e| e == name) => Grammar::NONE,
            None => return (words, dir),
        };
        // `command` with `v` or `V` among its options looks a program up; it runs nothing.
        if name == "command" && looks_up(&words[1..]) {
            return (words, dir);
        }
        let (rest, chdir) = grammar.strip(&words);
        if let Some(chdir) = chdir {
            dir = resolve(&dir, &chdir);
        }
        words = rest;
    }
}

/// Whether `command`'s options include `v` or `V`.
fn looks_up(args: &[String]) -> bool {
    args.iter()
        .take_while(|a| a.starts_with('-') && *a != "--")
        .any(|a| !a.starts_with("--") && a.contains(['v', 'V']))
}

/// `NAME=value`, as a shell or `env` reads it.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// A wrapper's options: which take a value, so their value isn't taken for the command. The GNU
/// and BSD versions' options together.
#[derive(Debug, Clone, Copy)]
struct Grammar {
    /// Short options that take a value, attached (`-n5`) or as the next word.
    short_with_value: &'static str,
    /// Long options that take a value as the next word when it isn't attached with `=`.
    long_with_value: &'static [&'static str],
    /// Its options are whole words after one dash (`arch -arch x86_64`, `arch -arm64`); these take
    /// a value.
    words_with_value: Option<&'static [&'static str]>,
    /// Arguments of its own after its options (`timeout`'s duration).
    positional: usize,
}

impl Grammar {
    const NONE: Self = Self {
        short_with_value: "",
        long_with_value: &[],
        words_with_value: None,
        positional: 0,
    };

    /// The words after this wrapper, its options and its own arguments. `env -S` splits its
    /// string into words ahead of the rest.
    fn strip(self, words: &[String]) -> (Vec<String>, Option<String>) {
        let mut i = 1;
        let mut split = None;
        let mut chdir = None;
        while let Some(word) = words.get(i) {
            if word == "--" {
                i += 1;
                break;
            }
            if !word.starts_with('-') || word == "-" {
                break;
            }
            if let Some(with_value) = self.words_with_value {
                i += if with_value.contains(&word.as_str()) {
                    2
                } else {
                    1
                };
                continue;
            }
            if let Some(long) = word.strip_prefix("--") {
                let (key, attached) = match long.split_once('=') {
                    Some((key, value)) => (key, Some(value)),
                    None => (long, None),
                };
                let value = attached.or_else(|| {
                    self.long_with_value
                        .contains(&key)
                        .then(|| words.get(i + 1).map(String::as_str))
                        .flatten()
                });
                match key {
                    "split-string" => split = value.map(str::to_owned),
                    "chdir" => chdir = value.map(str::to_owned),
                    _ => {}
                }
                i += if attached.is_none() && self.long_with_value.contains(&key) {
                    2
                } else {
                    1
                };
                continue;
            }
            let mut step = 1;
            for (at, c) in word.char_indices().skip(1) {
                if self.short_with_value.contains(c) {
                    let attached = &word[at + c.len_utf8()..];
                    let value = if attached.is_empty() {
                        step = 2;
                        words.get(i + 1).map(String::as_str)
                    } else {
                        Some(attached)
                    };
                    match c {
                        'S' => split = value.map(str::to_owned),
                        'C' => chdir = value.map(str::to_owned),
                        _ => {}
                    }
                    break;
                }
            }
            i += step;
        }
        i += self.positional;
        let rest = words.get(i..).unwrap_or_default();
        let words = match split {
            Some(string) => {
                let mut expanded = shell_words::split(&string).unwrap_or_else(|_| vec![string]);
                expanded.extend_from_slice(rest);
                expanded
            }
            None => rest.to_vec(),
        };
        (words, chdir)
    }
}

/// The built-in wrappers, by name wherever they live.
fn grammar(name: &str) -> Option<Grammar> {
    let g = |short_with_value, long_with_value| Grammar {
        short_with_value,
        long_with_value,
        ..Grammar::NONE
    };
    Some(match name {
        "env" => g("uCSP", &["unset", "chdir", "split-string"]),
        "exec" => g("a", &[]),
        "command" | "nohup" | "setsid" => Grammar::NONE,
        "time" => g("fo", &["format", "output"]),
        "nice" => g("n", &["adjustment"]),
        "timeout" | "gtimeout" => Grammar {
            positional: 1,
            ..g("ks", &["kill-after", "signal"])
        },
        "xargs" => g(
            "adEIJLnPRSs",
            &[
                "arg-file",
                "delimiter",
                "eof",
                "max-args",
                "max-procs",
                "max-lines",
                "max-chars",
                "process-slot-var",
            ],
        ),
        "stdbuf" => g("ioe", &["input", "output", "error"]),
        "ionice" => g("cn", &["class", "classdata"]),
        "taskpolicy" => g("cdgtlp", &[]),
        "arch" => Grammar {
            words_with_value: Some(&["-arch", "-e", "-d"]),
            ..Grammar::NONE
        },
        _ => return None,
    })
}

/// Rewrites a program path, resolved from `dir`, into the project relative to its root, so `./bin/x`, `bin/x` and its
/// absolute path all read `bin/x`. A bare name, or a path outside the project, stays as it is.
fn normalise_program(words: &mut [String], dir: &str, root: &str) {
    let Some(program) = words.first_mut() else {
        return;
    };
    if !program.contains('/') {
        return;
    }
    let absolute = resolve(dir, program);
    let root = lexical(root);
    *program = match absolute.strip_prefix(&format!("{root}/")) {
        Some(relative) => relative.to_owned(),
        None => absolute,
    };
}

/// `path` from `dir`, as an absolute path resolved by name.
pub(crate) fn resolve(dir: &str, path: &str) -> String {
    if path.starts_with('/') {
        lexical(path)
    } else {
        lexical(&format!("{dir}/{path}"))
    }
}

/// An absolute path with `.` and `..` resolved by name, not by the file system.
fn lexical(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

pub(crate) fn basename(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

/// The string a shell's `-c` runs: options come first (`-` or `+`, clustered, with `-o`, `-O` and
/// `--rcfile` taking a value), one of them carries `c` (`-c`, `-lc`), and the first word after them
/// is the string.
pub(crate) fn shell_script(args: &[String]) -> Option<String> {
    let (dash_c, _, operands) = shell_options(args);
    dash_c.then(|| args.get(operands).cloned()).flatten()
}

/// A shell reads its script from stdin: no `-c` string, and `-s` or no script file.
pub(crate) fn reads_stdin(args: &[String]) -> bool {
    let (dash_c, dash_s, operands) = shell_options(args);
    !dash_c && (dash_s || operands >= args.len())
}

/// A shell's options: whether they include `-c` and `-s`, and where its operands start.
fn shell_options(args: &[String]) -> (bool, bool, usize) {
    let mut dash_c = false;
    let mut dash_s = false;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        i += 1;
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            if matches!(long, "rcfile" | "init-file") {
                i += 1;
            }
            continue;
        }
        let Some(flags) = arg.strip_prefix(['-', '+']).filter(|f| !f.is_empty()) else {
            i -= 1;
            break;
        };
        dash_c |= arg.starts_with('-') && flags.contains('c');
        dash_s |= arg.starts_with('-') && flags.contains('s');
        if flags.contains(['o', 'O']) {
            i += 1;
        }
    }
    (dash_c, dash_s, i)
}

/// The argv joined with single spaces, quoting any argument that contains whitespace or quotes.
fn join(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            if w.chars()
                .any(|c| c.is_whitespace() || c == '\'' || c == '"')
            {
                format!("'{}'", w.replace('\'', r"'\''"))
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
