//! The hook's spotter (ARCHITECTURE, Class files): finds the programs a command string runs,
//! without understanding it. It decides no class: it says which programs run, where each one's
//! simple command starts and ends, and the directory it runs in. Pure.

use std::ops::Range;

use crate::matching::{
    Machine, SHELLS, basename, reads_stdin, resolve, shell_script, strip_wrappers,
};

/// One program the string runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spotted {
    /// The program word as written, dequoted, after assignments, wrappers and launchers; from the
    /// project's root when `rooted`.
    pub program: String,
    /// The program's path began with a command that prints the project's root (`$(repo-root)/`).
    pub rooted: bool,
    /// The directory it runs in, as far as the spotter could follow it.
    pub dir: String,
    /// The simple command to replace in a refusal, as a byte range of the string. For a program in
    /// a `sh -c` string, the whole shell command.
    pub span: Range<usize>,
    /// The command's leading assignments, which a refusal moves after `--` with `env`.
    pub assignments: Vec<Range<usize>>,
}

/// The programs `script` runs, in order, starting in `cwd`.
#[must_use]
pub fn spot(script: &str, cwd: &str, machine: &Machine) -> Vec<Spotted> {
    let mut found = Vec::new();
    Spotter {
        machine,
        found: &mut found,
    }
    .list(script, 0, Place::new(resolve("/", cwd)), None);
    found
}

/// The refusal's suggestion: `script` with `spotted`'s simple command run through maitred, its
/// assignments moved after `--` with `env`, and `queue` in front when the class file names one.
#[must_use]
pub fn suggest(script: &str, spotted: &Spotted, queue: Option<&str>) -> String {
    let Range { start, end } = spotted.span.clone();
    let after_assignments = spotted.assignments.last().map_or(start, |a| a.end);
    let rest = script[after_assignments..end].trim_start();
    let mut command = String::new();
    if let Some(queue) = queue {
        command.push_str(queue);
        command.push(' ');
    }
    command.push_str("maitred -- ");
    if !spotted.assignments.is_empty() {
        command.push_str("env ");
        for a in &spotted.assignments {
            command.push_str(&script[a.clone()]);
            command.push(' ');
        }
    }
    command.push_str(rest);
    format!("{}{command}{}", &script[..start], &script[end..])
}

// The lexer: words and operators, with byte ranges into the string being read.

#[derive(Debug, Clone)]
struct Word {
    /// Dequoted, with substitutions left as written.
    text: String,
    span: Range<usize>,
    /// No expansion in it (no `$`, backquote or leading `~`): its text is what runs.
    plain: bool,
    /// Where an unquoted `=` falls in `text`, if the name before it was unquoted too.
    unquoted_eq: Option<usize>,
    /// Command substitutions in it, as ranges of their insides.
    substitutions: Vec<Range<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// `;`, `&`, `&&`, `||`, `|`, `|&`, `&|`, `&!` and a newline: a new command follows.
    Separator,
    Open,
    Close,
    /// `;;`, `;&` and `;;&`: a `case` branch ends.
    CaseEnd,
}

#[derive(Debug, Clone)]
enum Token {
    Word(Word),
    Op(Op),
    /// Substitutions to spot that belong to no word: a redirection's target.
    Substitutions(Vec<Range<usize>>),
    /// A heredoc, where its `<<` falls among the command's words; its body is filled in once the
    /// lexer reaches it.
    Heredoc(Body),
}

/// A heredoc's body, the shell's stdin when it names no other descriptor; or a redirection's
/// substitutions alone, with no body.
#[derive(Debug, Clone, Default)]
struct Body {
    span: Range<usize>,
    stdin: bool,
    /// Its substitutions, when its delimiter wasn't quoted.
    substitutions: Vec<Range<usize>>,
}

struct Heredoc {
    delimiter: String,
    quoted: bool,
    strip_tabs: bool,
    /// The index of its `Token::Heredoc`.
    token: usize,
}

fn lex(s: &str) -> Vec<Token> {
    let bytes = s.as_bytes();
    let mut tokens = Vec::new();
    let mut heredocs: Vec<Heredoc> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\\' if bytes.get(i + 1) == Some(&b'\n') => i += 2,
            b'\n' => {
                tokens.push(Token::Op(Op::Separator));
                i += 1;
                for heredoc in heredocs.drain(..) {
                    let (end, span, substitutions) = heredoc_body(s, i, &heredoc);
                    if let Token::Heredoc(body) = &mut tokens[heredoc.token] {
                        body.span = span;
                        body.substitutions = substitutions;
                    }
                    i = end;
                }
            }
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b';' => {
                // `;;&` lexes as `;;` and then `&`, which a case's patterns ignore.
                let (op, len) = match bytes.get(i + 1) {
                    Some(b';' | b'&') => (Op::CaseEnd, 2),
                    _ => (Op::Separator, 1),
                };
                tokens.push(Token::Op(op));
                i += len;
            }
            // `&>`; a second `>` (`&>>`) lexes as a redirection of its own with the target.
            b'&' if bytes.get(i + 1) == Some(&b'>') => i = redirect_target(s, i + 2, &mut tokens),
            b'&' | b'|' => {
                let pair = matches!(bytes.get(i + 1), Some(b'&' | b'|' | b'!'));
                tokens.push(Token::Op(Op::Separator));
                i += if pair { 2 } else { 1 };
            }
            b'(' => {
                tokens.push(Token::Op(Op::Open));
                i += 1;
            }
            b')' => {
                tokens.push(Token::Op(Op::Close));
                i += 1;
            }
            b'<' | b'>' => {
                // A word of digits just before is the descriptor, not a word.
                let mut descriptor = None;
                if let Some(Token::Word(w)) = tokens.last()
                    && w.span.end == i
                    && w.plain
                    && w.text.bytes().all(|c| c.is_ascii_digit())
                {
                    descriptor = Some(w.text.clone());
                    tokens.pop();
                }
                if s[i..].starts_with("<<<") {
                    i = redirect_target(s, i + 3, &mut tokens);
                } else if s[i..].starts_with("<<") {
                    let strip_tabs = bytes.get(i + 2) == Some(&b'-');
                    i += if strip_tabs { 3 } else { 2 };
                    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
                        i += 1;
                    }
                    let (word, end) = word(s, i);
                    let raw = &s[word.span.clone()];
                    heredocs.push(Heredoc {
                        delimiter: word.text,
                        quoted: raw.contains(['\'', '"', '\\']),
                        strip_tabs,
                        token: tokens.len(),
                    });
                    tokens.push(Token::Heredoc(Body {
                        stdin: descriptor.is_none_or(|d| d.parse() == Ok(0)),
                        ..Body::default()
                    }));
                    i = end;
                } else {
                    let len = match (bytes[i], bytes.get(i + 1)) {
                        (_, Some(b'&' | b'|' | b'>')) | (b'<', Some(b'<')) => 2,
                        _ => 1,
                    };
                    i = redirect_target(s, i + len, &mut tokens);
                }
            }
            _ => {
                let (word, end) = word(s, i);
                tokens.push(Token::Word(word));
                i = end;
            }
        }
    }
    tokens
}

/// A redirection's target: skipped as a word, but a substitution in it still runs.
fn redirect_target(s: &str, mut i: usize, tokens: &mut Vec<Token>) -> usize {
    let bytes = s.as_bytes();
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if i >= bytes.len() || b"\n;&|()<>".contains(&bytes[i]) {
        return i;
    }
    let (target, end) = word(s, i);
    if !target.substitutions.is_empty() {
        tokens.push(Token::Substitutions(target.substitutions));
    }
    end
}

/// A heredoc's body from `start`: where its delimiter's line ends, the body itself, and the
/// substitutions in it when its delimiter wasn't quoted.
fn heredoc_body(
    s: &str,
    start: usize,
    heredoc: &Heredoc,
) -> (usize, Range<usize>, Vec<Range<usize>>) {
    let mut substitutions = Vec::new();
    let mut i = start;
    while i < s.len() {
        let line_end = s[i..].find('\n').map_or(s.len(), |n| i + n);
        let line = &s[i..line_end];
        let line = if heredoc.strip_tabs {
            line.trim_start_matches('\t')
        } else {
            line
        };
        let next = (line_end + 1).min(s.len());
        if line == heredoc.delimiter {
            return (next, start..i, substitutions);
        }
        if !heredoc.quoted {
            let mut j = i;
            while j < line_end {
                match s.as_bytes()[j] {
                    b'\\' => j += 2,
                    b'$' if s[j..].starts_with("$(") => {
                        let close = closing_paren(s, j + 2);
                        substitutions.push(j + 2..close);
                        // The `)` itself is stepped over below.
                        j = close;
                    }
                    b'`' => {
                        let close = closing_backquote(s, j + 1);
                        substitutions.push(j + 1..close);
                        j = close + 1;
                    }
                    _ => j += 1,
                }
            }
        }
        i = next;
    }
    (s.len(), start..s.len(), substitutions)
}

/// One word from `start`, and where it ends.
fn word(s: &str, start: usize) -> (Word, usize) {
    let bytes = s.as_bytes();
    let mut w = Word {
        text: String::new(),
        span: start..start,
        plain: bytes.get(start) != Some(&b'~'),
        unquoted_eq: None,
        substitutions: Vec::new(),
    };
    let mut quoted_name = false;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' | b';' | b'&' | b'|' | b'(' | b')' | b'<' | b'>' => break,
            b'\\' => {
                match s[i + 1..].chars().next() {
                    // A backslash-newline joins two lines.
                    Some('\n') | None => {}
                    Some(c) => w.text.push(c),
                }
                i += 1 + s[i + 1..].chars().next().map_or(0, char::len_utf8);
            }
            b'\'' => {
                quoted_name = true;
                let close = s[i + 1..].find('\'').map_or(s.len(), |n| i + 1 + n);
                w.text.push_str(&s[i + 1..close]);
                i = (close + 1).min(s.len());
            }
            b'"' => {
                quoted_name = true;
                i = double_quoted(s, i + 1, &mut w);
            }
            b'$' if s[i..].starts_with("$'") => {
                quoted_name = true;
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != b'\'' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                let close = j.min(s.len());
                w.text.push_str(&s[i + 2..close]);
                i = (close + 1).min(s.len());
            }
            b'$' if s[i..].starts_with("$(") => {
                w.plain = false;
                let close = closing_paren(s, i + 2);
                w.substitutions.push(i + 2..close);
                let end = (close + 1).min(s.len());
                w.text.push_str(&s[i..end]);
                i = end;
            }
            b'`' => {
                w.plain = false;
                let close = closing_backquote(s, i + 1);
                w.substitutions.push(i + 1..close);
                let end = (close + 1).min(s.len());
                w.text.push_str(&s[i..end]);
                i = end;
            }
            b'$' => {
                w.plain = false;
                w.text.push('$');
                i += 1;
            }
            b'=' if w.unquoted_eq.is_none() && !quoted_name => {
                w.unquoted_eq = Some(w.text.len());
                w.text.push('=');
                i += 1;
            }
            _ => {
                let c = s[i..].chars().next().unwrap_or_default();
                w.text.push(c);
                i += c.len_utf8();
            }
        }
    }
    w.span.end = i;
    (w, i)
}

/// The inside of double quotes from `i`, into `w`; where it ends. `$(…)` and backquotes still run.
fn double_quoted(s: &str, mut i: usize, w: &mut Word) -> usize {
    let bytes = s.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return i + 1,
            b'\\' => {
                match s[i + 1..].chars().next() {
                    Some('\n') | None => {}
                    Some(c @ ('"' | '\\' | '$' | '`')) => w.text.push(c),
                    Some(c) => {
                        w.text.push('\\');
                        w.text.push(c);
                    }
                }
                i += 1 + s[i + 1..].chars().next().map_or(0, char::len_utf8);
            }
            b'$' if s[i..].starts_with("$(") => {
                w.plain = false;
                let close = closing_paren(s, i + 2);
                w.substitutions.push(i + 2..close);
                let end = (close + 1).min(s.len());
                w.text.push_str(&s[i..end]);
                i = end;
            }
            b'`' => {
                w.plain = false;
                let close = closing_backquote(s, i + 1);
                w.substitutions.push(i + 1..close);
                let end = (close + 1).min(s.len());
                w.text.push_str(&s[i..end]);
                i = end;
            }
            b'$' => {
                w.plain = false;
                w.text.push('$');
                i += 1;
            }
            _ => {
                let c = s[i..].chars().next().unwrap_or_default();
                w.text.push(c);
                i += c.len_utf8();
            }
        }
    }
    s.len()
}

/// The `)` closing a `$(` whose inside starts at `i`, skipping quotes and nested parentheses; the
/// end of the string if there is none.
fn closing_paren(s: &str, mut i: usize) -> usize {
    let bytes = s.as_bytes();
    let mut depth = 0;
    // Inside `case … esac`, an unmatched `)` ends a pattern, not the substitution.
    let mut cases = 0usize;
    let starts_word = |at: usize| at == 0 || b" \t\n;&|(".contains(&bytes[at - 1]);
    let is_word = |at: usize, word: &str| {
        s[at..].starts_with(word)
            && bytes
                .get(at + word.len())
                .is_none_or(|c| b" \t\n;&|)".contains(c))
    };
    while i < bytes.len() {
        if starts_word(i) {
            if is_word(i, "case") {
                cases += 1;
            } else if is_word(i, "esac") {
                cases = cases.saturating_sub(1);
            }
        }
        match bytes[i] {
            b'\\' => i += 1,
            b'\'' => i = s[i + 1..].find('\'').map_or(s.len(), |n| i + 1 + n),
            b'"' => {
                let mut scratch = Word {
                    text: String::new(),
                    span: 0..0,
                    plain: true,
                    unquoted_eq: None,
                    substitutions: Vec::new(),
                };
                i = double_quoted(s, i + 1, &mut scratch) - 1;
            }
            b'(' => depth += 1,
            b')' if depth == 0 && cases == 0 => return i,
            b')' if depth == 0 => {}
            b')' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    s.len()
}

/// The backquote closing one whose inside starts at `i`; the end of the string if there is none.
fn closing_backquote(s: &str, mut i: usize) -> usize {
    let bytes = s.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => return i,
            _ => i += 1,
        }
    }
    s.len()
}

// The spotter proper.

/// Keywords after which the next word is in command position.
const LEADS: [&str; 10] = [
    "if", "then", "else", "elif", "while", "until", "do", "!", "{", "time",
];
/// Keywords that close a construct and run nothing.
const CLOSES: [&str; 4] = ["fi", "done", "esac", "}"];

struct Spotter<'m, 'f> {
    machine: &'m Machine,
    found: &'f mut Vec<Spotted>,
}

/// Where the shell is, as far as the spotter can follow it: its directory and the ones `pushd`
/// left for `popd`.
#[derive(Clone)]
struct Place {
    dir: String,
    pushed: Vec<String>,
}

impl Place {
    fn new(dir: String) -> Self {
        Self {
            dir,
            pushed: Vec::new(),
        }
    }

    /// Follows a literal cd, pushd or popd; one it can't read leaves the directory be.
    fn follow(&mut self, name: &str, words: &[Word], stripped: &[String], runs_in: &str) {
        match name {
            "cd" | "pushd" if stripped.len() >= 2 => {
                if name == "pushd" {
                    self.pushed.push(self.dir.clone());
                }
                let arg_at = words.len() - stripped.len() + 1;
                if let Some(arg) = words
                    .get(arg_at)
                    .filter(|a| a.plain && !a.text.is_empty() && !a.text.starts_with('-'))
                {
                    self.dir = resolve(runs_in, &arg.text);
                }
            }
            "popd" if stripped.len() == 1 => {
                if let Some(dir) = self.pushed.pop() {
                    self.dir = dir;
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Command,
    /// Inside a `case`, reading a branch's pattern up to its `)`.
    Pattern,
}

impl Spotter<'_, '_> {
    /// Spots a list read from `s`, whose byte 0 is `base` in the string being refused. `wrap` is
    /// the shell command whose script `s` is, if it is one.
    fn list(&mut self, s: &str, base: usize, mut place: Place, wrap: Option<&Spotted>) {
        // A subshell's place ends with it.
        let mut outer: Vec<Place> = Vec::new();
        let mut words: Vec<Word> = Vec::new();
        let mut bodies: Vec<Body> = Vec::new();
        let mut mode = Mode::Command;
        for token in lex(s) {
            match (mode, token) {
                (Mode::Pattern, Token::Word(w)) => {
                    if w.text == "esac" {
                        mode = Mode::Command;
                    }
                }
                (Mode::Pattern, Token::Op(Op::Close)) => mode = Mode::Command,
                (Mode::Pattern, Token::Op(_)) => {}
                (Mode::Pattern, Token::Substitutions(subs)) => {
                    for sub in subs {
                        self.list(&s[sub.clone()], base + sub.start, place.clone(), wrap);
                    }
                }
                // A redirection's substitutions run with its command, so they come after its
                // program, as a heredoc's do.
                (Mode::Command, Token::Substitutions(substitutions)) => bodies.push(Body {
                    substitutions,
                    ..Body::default()
                }),
                (Mode::Pattern, Token::Heredoc(body)) => {
                    self.bodies(s, base, &place, &[body], None, wrap);
                }
                (Mode::Command, Token::Heredoc(body)) => bodies.push(body),
                (Mode::Command, Token::Word(w)) => words.push(w),
                (Mode::Command, Token::Op(op)) => {
                    let words = std::mem::take(&mut words);
                    let bodies = std::mem::take(&mut bodies);
                    if self.command(s, base, &mut place, &words, &bodies, wrap) {
                        mode = Mode::Pattern;
                        continue;
                    }
                    match op {
                        Op::Open => outer.push(place.clone()),
                        Op::Close => {
                            if let Some(before) = outer.pop() {
                                place = before;
                            }
                        }
                        Op::CaseEnd => mode = Mode::Pattern,
                        Op::Separator => {}
                    }
                }
            }
        }
        if mode == Mode::Command {
            self.command(s, base, &mut place, &words, &bodies, wrap);
        }
    }

    /// Spots one simple command and the heredocs it reads. True when it opens a `case`, whose
    /// patterns come next.
    fn command(
        &mut self,
        s: &str,
        base: usize,
        place: &mut Place,
        words: &[Word],
        bodies: &[Body],
        wrap: Option<&Spotted>,
    ) -> bool {
        let (opens_case, script) = self.simple(s, base, place, words, bodies, wrap);
        self.bodies(s, base, place, bodies, script, wrap);
        opens_case
    }

    /// The substitutions in each heredoc but `script`, the one a shell reads as its script.
    fn bodies(
        &mut self,
        s: &str,
        base: usize,
        place: &Place,
        bodies: &[Body],
        script: Option<usize>,
        wrap: Option<&Spotted>,
    ) {
        for (at, body) in bodies.iter().enumerate() {
            if Some(at) != script {
                for sub in &body.substitutions {
                    self.list(&s[sub.clone()], base + sub.start, place.clone(), wrap);
                }
            }
        }
    }

    /// Spots one simple command: true when it opens a `case`, and the heredoc a shell reads as
    /// its script, if one does.
    fn simple(
        &mut self,
        s: &str,
        base: usize,
        place: &mut Place,
        words: &[Word],
        bodies: &[Body],
        wrap: Option<&Spotted>,
    ) -> (bool, Option<usize>) {
        let mut words = words;
        // Keywords before the command.
        while let Some(first) = words.first().filter(|w| w.plain) {
            let keyword = first.text.as_str();
            if keyword == "case" {
                self.substitutions(s, base, place, words, wrap);
                return (true, None);
            }
            if matches!(keyword, "for" | "select") || CLOSES.contains(&keyword) {
                self.substitutions(s, base, place, words, wrap);
                return (false, None);
            }
            if keyword == "repeat" {
                words = words.get(2..).unwrap_or_default();
            } else if keyword == "function" {
                // Its name; the body that follows is a group or a subshell.
                words = words.get(2..).unwrap_or_default();
            } else if keyword == "coproc" {
                // `coproc NAME { … }` names it; `coproc command` doesn't.
                let named = words.get(2).is_some_and(|w| w.text == "{");
                words = &words[if named { 2 } else { 1 }..];
            } else if keyword == "time" && words.get(1).is_some_and(|w| w.text == "-p") {
                words = &words[2..];
            } else if LEADS.contains(&keyword) {
                words = &words[1..];
            } else {
                break;
            }
        }
        let (Some(first), Some(last)) = (words.first(), words.last()) else {
            return (false, None);
        };
        let assignments: Vec<&Word> = words
            .iter()
            .take_while(|w| w.unquoted_eq.is_some_and(|at| is_name(&w.text[..at])))
            .collect();
        let texts: Vec<String> = words[assignments.len()..]
            .iter()
            .map(|w| w.text.clone())
            .collect();
        let (mut stripped, runs_in) = strip_wrappers(texts, &self.machine.wrappers, &place.dir);
        if let Some(launcher) = self
            .machine
            .launchers
            .iter()
            .find(|l| !l.is_empty() && stripped.starts_with(l))
        {
            stripped.drain(..launcher.len());
        }
        let mut script = None;
        if let Some(program) = stripped.first() {
            let from_root = self.path_from_root(program);
            let spotted = Spotted {
                rooted: from_root.is_some(),
                program: from_root.unwrap_or_else(|| program.clone()),
                dir: runs_in.clone(),
                span: wrap.map_or(base + first.span.start..base + last.span.end, |w| {
                    w.span.clone()
                }),
                assignments: wrap.map_or_else(
                    || {
                        assignments
                            .iter()
                            .map(|a| base + a.span.start..base + a.span.end)
                            .collect()
                    },
                    |w| w.assignments.clone(),
                ),
            };
            let name = basename(program);
            self.found.push(spotted.clone());
            place.follow(name, words, &stripped, &runs_in);
            // A shell's script, recursively, in the shell's directory: its -c string, or the
            // heredoc it reads on stdin.
            let outer = wrap.cloned().unwrap_or(spotted);
            if SHELLS.contains(&name) {
                if let Some(string) = shell_script(&stripped[1..]) {
                    self.list(&string, 0, Place::new(runs_in), Some(&outer));
                } else if reads_stdin(&stripped[1..])
                    && let Some(at) = bodies.iter().rposition(|b| b.stdin)
                {
                    let body = bodies[at].span.clone();
                    self.list(
                        &s[body.clone()],
                        base + body.start,
                        Place::new(runs_in),
                        Some(&outer),
                    );
                    script = Some(at);
                }
            }
        }
        self.substitutions(s, base, place, words, wrap);
        (false, script)
    }

    /// The rest of a program path that starts with a substitution printing the project's root,
    /// with `.` and `..` resolved; none when it leaves the project.
    fn path_from_root(&self, program: &str) -> Option<String> {
        let (inner, after) = if let Some(rest) = program.strip_prefix("$(") {
            rest.split_once(')')?
        } else {
            program.strip_prefix('`')?.split_once('`')?
        };
        let inner = inner.trim();
        let prints_root = inner == "git rev-parse --show-toplevel"
            || self.machine.root_commands.iter().any(|c| c == inner);
        let mut parts: Vec<&str> = Vec::new();
        for part in after.strip_prefix('/')?.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                _ => parts.push(part),
            }
        }
        (prints_root && !parts.is_empty()).then(|| parts.join("/"))
    }

    /// The command substitutions in `words`, each a list of its own in the current place.
    fn substitutions(
        &mut self,
        s: &str,
        base: usize,
        place: &Place,
        words: &[Word],
        wrap: Option<&Spotted>,
    ) {
        for sub in words.iter().flat_map(|w| w.substitutions.clone()) {
            self.list(&s[sub.clone()], base + sub.start, place.clone(), wrap);
        }
    }
}

/// A shell variable's name.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod props;
#[cfg(test)]
mod tests;
