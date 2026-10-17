//! A project's class file (ARCHITECTURE, Class files): one rule per line, `name | pattern | class |
//! why | options`, read on every run. Pure: the caller reads the file and knows the machine's pools.

use std::collections::BTreeSet;

use regex::Regex;

use crate::Pool;

/// A parsed class file.
#[derive(Debug, Clone)]
pub struct ClassFile {
    /// The optional first line, `queue: docker compose exec app`: the project's jobs are queued
    /// inside a container.
    pub queue: Option<String>,
    /// In the file's order: the first rule whose pattern matches wins.
    pub rules: Vec<Rule>,
    /// Rules left out because they ask for a pool this machine doesn't declare, so one file serves a
    /// laptop, a container and CI.
    pub skipped: Vec<Skipped>,
}

/// One rule.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Its line in the file, counting from 1.
    pub line: usize,
    pub name: String,
    /// The literal path the pattern starts with: what `classes guard` and the spotter look for.
    pub program: String,
    pub pattern: Regex,
    pub class: RuleClass,
    pub why: String,
    pub options: Options,
}

/// A rule's class: a ticket's three, or `none`, which runs the program directly with no ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleClass {
    Light,
    Heavy,
    Quiet,
    None,
}

/// A rule's options, each unset unless the rule gives it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// A program path relative to the project root.
    pub checker: Option<String>,
    pub cpu_millicores: Option<u64>,
    pub mem_mb: Option<u64>,
    /// One slot from each pool named.
    pub slots: BTreeSet<Pool>,
    pub leftovers: Leftovers,
}

/// What happens to processes still in the job's session when its main process exits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Leftovers {
    #[default]
    Stop,
    Keep,
}

/// A rule left out for a pool this machine doesn't declare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub line: usize,
    pub rule: String,
    pub pool: Pool,
}

/// Why the file is invalid, and on which line. An invalid file makes every command heavy, with a
/// warning naming the line; nothing is refused for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub line: usize,
    pub why: Why,
}

/// What is wrong with an invalid line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// Not four or five fields separated by ` | `.
    Fields,
    NoName,
    DuplicateName(String),
    NoWhy,
    /// The pattern doesn't compile; the regex crate's message.
    BadPattern(String),
    /// The pattern doesn't start with `^` and its program's literal path.
    NoProgram,
    UnknownClass(String),
    UnknownOption(String),
    BadValue {
        key: String,
        value: String,
    },
    RepeatedOption(String),
    EmptyQueue,
}

/// Parses a class file. `pools` are the pools this machine declares.
///
/// # Errors
///
/// The first invalid line.
pub fn parse(text: &str, pools: &BTreeSet<Pool>) -> Result<ClassFile, Invalid> {
    let mut file = ClassFile {
        queue: None,
        rules: Vec::new(),
        skipped: Vec::new(),
    };
    let mut names = BTreeSet::new();
    let mut first = true;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let invalid = |why| Invalid { line, why };
        // Only the first line that isn't a comment may say where the queue lives.
        if first && let Some(queue) = trimmed.strip_prefix("queue:") {
            first = false;
            let queue = queue.trim();
            if queue.is_empty() {
                return Err(invalid(Why::EmptyQueue));
            }
            file.queue = Some(queue.to_owned());
            continue;
        }
        first = false;
        let rule = rule(line, raw).map_err(invalid)?;
        if !names.insert(rule.name.clone()) {
            return Err(invalid(Why::DuplicateName(rule.name)));
        }
        match rule.options.slots.iter().find(|p| !pools.contains(*p)) {
            Some(pool) => file.skipped.push(Skipped {
                line,
                rule: rule.name,
                pool: pool.clone(),
            }),
            None => file.rules.push(rule),
        }
    }
    Ok(file)
}

/// One rule's line. A bar inside a pattern needs no escaping: the separator has spaces around it.
/// Padded so an empty first or last field still has its separator.
fn rule(line: usize, text: &str) -> Result<Rule, Why> {
    let padded = format!(" {text} ");
    let fields: Vec<&str> = padded.split(" | ").map(str::trim).collect();
    if !(3..=5).contains(&fields.len()) {
        return Err(Why::Fields);
    }
    if fields[0].is_empty() {
        return Err(Why::NoName);
    }
    // A rule with no why field at all is missing its why, not malformed.
    let why = fields.get(3).copied().unwrap_or("");
    if why.is_empty() {
        return Err(Why::NoWhy);
    }
    let (name, pattern, class) = (fields[0], fields[1], fields[2]);
    let class = match class {
        "light" => RuleClass::Light,
        "heavy" => RuleClass::Heavy,
        "quiet" => RuleClass::Quiet,
        "none" => RuleClass::None,
        other => return Err(Why::UnknownClass(other.to_owned())),
    };
    let compiled = Regex::new(pattern).map_err(|e| Why::BadPattern(e.to_string()))?;
    Ok(Rule {
        line,
        name: name.to_owned(),
        program: program(pattern).ok_or(Why::NoProgram)?,
        pattern: compiled,
        class,
        why: why.to_owned(),
        options: options(fields.get(4).copied().unwrap_or(""))?,
    })
}

/// The literal path a pattern starts with, after its `^`: letters, digits and `_ / + -`, and any
/// punctuation escaped with a backslash. It must end where the program's name does: at the end of
/// the pattern, a space, `(`, `$`, `[ ]`, `\s` or `\b`. None otherwise, so a pattern can't claim
/// one program while matching another (`^bin/x[a-z]`, or `^bin/x|evil`, whose other half matches
/// anywhere).
fn program(pattern: &str) -> Option<String> {
    let rest = pattern.strip_prefix('^')?;
    let mut path = String::new();
    let mut chars = rest.char_indices().peekable();
    let end = loop {
        match chars.next() {
            Some((_, c)) if c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '+' | '-') => {
                path.push(c);
            }
            Some((at, '\\')) => match chars.peek() {
                Some(&(_, escaped)) if escaped.is_ascii_punctuation() => {
                    path.push(escaped);
                    chars.next();
                }
                _ => break at,
            },
            Some((at, _)) => break at,
            None => break rest.len(),
        }
    };
    let after = &rest[end..];
    let ends_here = after.is_empty()
        || [" ", "(", "$", "[ ]", "\\s", "\\b"]
            .iter()
            .any(|boundary| after.starts_with(boundary));
    (!path.is_empty() && ends_here).then_some(path)
}

/// A rule's options: space-separated `key=value` pairs, each key at most once (`slot` once per
/// pool).
fn options(text: &str) -> Result<Options, Why> {
    let mut options = Options::default();
    let mut seen = BTreeSet::new();
    for pair in text.split_whitespace() {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(Why::UnknownOption(pair.to_owned()));
        };
        let bad = || Why::BadValue {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        if key != "slot" && !seen.insert(key) {
            return Err(Why::RepeatedOption(key.to_owned()));
        }
        match key {
            "checker" => options.checker = Some(checker(value).ok_or_else(bad)?),
            "cpu" => options.cpu_millicores = Some(millicores(value).ok_or_else(bad)?),
            "mem" => options.mem_mb = Some(megabytes(value).ok_or_else(bad)?),
            "leftovers" => {
                options.leftovers = match value {
                    "stop" => Leftovers::Stop,
                    "keep" => Leftovers::Keep,
                    _ => return Err(bad()),
                };
            }
            "slot" => {
                if value.is_empty() {
                    return Err(bad());
                }
                if !options.slots.insert(Pool(value.to_owned())) {
                    return Err(Why::RepeatedOption(pair.to_owned()));
                }
            }
            _ => return Err(Why::UnknownOption(key.to_owned())),
        }
    }
    Ok(options)
}

/// A checker's path, relative to the project root and inside it.
fn checker(value: &str) -> Option<String> {
    let inside =
        !value.is_empty() && !value.starts_with('/') && !value.split('/').any(|part| part == "..");
    inside.then(|| value.to_owned())
}

/// Cores, to at most three decimal places, as millicores; more than zero.
fn millicores(value: &str) -> Option<u64> {
    let (whole, fraction) = match value.split_once('.') {
        // A bare trailing dot isn't a decimal place.
        Some((_, "")) => return None,
        Some(parts) => parts,
        None => (value, ""),
    };
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    // `parse` alone would take a leading `+`; an empty part fails it anyway.
    if !digits(whole) || !digits(fraction) || fraction.len() > 3 {
        return None;
    }
    let padded = format!("{fraction:0<3}");
    let millis = whole
        .parse::<u64>()
        .ok()?
        .checked_mul(1_000)?
        .checked_add(padded.parse::<u64>().ok()?)?;
    (millis > 0).then_some(millis)
}

/// A whole number of megabytes (`M`) or gigabytes (`G`), as megabytes; more than zero.
fn megabytes(value: &str) -> Option<u64> {
    let (number, scale) = if let Some(n) = value.strip_suffix('M') {
        (n, 1)
    } else {
        (value.strip_suffix('G')?, 1_024)
    };
    // `parse` alone would take a leading `+`; an empty number fails it anyway.
    if !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mb = number.parse::<u64>().ok()?.checked_mul(scale)?;
    (mb > 0).then_some(mb)
}

#[cfg(test)]
mod tests;
