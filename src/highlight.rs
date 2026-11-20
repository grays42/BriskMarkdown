//! Tiny hand-written syntax highlighter for fenced code blocks.
//!
//! No regex, no crates, std only. Every scanner is a single forward pass over the bytes, so the
//! cost is linear. All delimiters are ASCII; bytes >= 0x80 are treated as identifier characters, so
//! tokens never split a UTF-8 sequence. `Out::push` additionally enforces ordering, non-overlap and
//! char boundaries, so a scanner bug can lose colour but never panic or corrupt ranges.

use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Tok {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Constant,
    Property,
    Tag,
    Meta,
    Inserted,
    Deleted,
    Variable,
}

#[allow(dead_code)]
impl Tok {
    pub const ALL: [Tok; 13] = [
        Tok::Keyword,
        Tok::String,
        Tok::Comment,
        Tok::Number,
        Tok::Type,
        Tok::Function,
        Tok::Constant,
        Tok::Property,
        Tok::Tag,
        Tok::Meta,
        Tok::Inserted,
        Tok::Deleted,
        Tok::Variable,
    ];

    /// CSS class suffix used by themes.
    pub fn name(self) -> &'static str {
        match self {
            Tok::Keyword => "keyword",
            Tok::String => "string",
            Tok::Comment => "comment",
            Tok::Number => "number",
            Tok::Type => "type",
            Tok::Function => "function",
            Tok::Constant => "constant",
            Tok::Property => "property",
            Tok::Tag => "tag",
            Tok::Meta => "meta",
            Tok::Inserted => "inserted",
            Tok::Deleted => "deleted",
            Tok::Variable => "variable",
        }
    }
}

/// Highlight `code` written in language `lang`. See the module docs; unknown languages give an
/// empty Vec. Ranges are sorted, non-overlapping and on char boundaries.
pub fn highlight(lang: &str, code: &str) -> Vec<(Range<usize>, Tok)> {
    let l = lang.trim().to_ascii_lowercase();
    let mut o = Out { code, v: Vec::new(), last: 0 };
    if code.is_empty() {
        return o.v;
    }
    match l.as_str() {
        "python" | "py" | "python3" | "py3" => python(code, &mut o),
        "bash" | "sh" | "shell" | "zsh" | "console" | "shell-session" | "shellsession" | "ksh" => shell(code, &mut o),
        "powershell" | "ps1" | "pwsh" | "ps" | "psm1" => powershell(code, &mut o),
        "json" | "jsonc" | "json5" => json(code, &mut o),
        "yaml" | "yml" => yaml(code, &mut o),
        "toml" => ini(code, &mut o, true),
        "ini" | "cfg" | "conf" => ini(code, &mut o, false),
        "html" | "htm" | "xml" | "svg" | "xhtml" | "vue" | "xsl" => markup(code, &mut o),
        "css" => css(code, &mut o, false),
        "scss" | "less" => css(code, &mut o, true),
        "sql" | "mysql" | "postgres" | "postgresql" | "pgsql" | "sqlite" | "tsql" => sql(code, &mut o),
        "diff" | "patch" => diff(code, &mut o),
        "dockerfile" | "docker" => docker(code, &mut o),
        "makefile" | "make" | "mk" => makefile(code, &mut o),
        "markdown" | "md" => markdown(code, &mut o),
        other => {
            if let Some(cfg) = lang_cfg(other) {
                clike(code, &cfg, &mut o);
            }
        }
    }
    o.v
}

// ---------------------------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------------------------

struct Out<'a> {
    code: &'a str,
    v: Vec<(Range<usize>, Tok)>,
    last: usize,
}

impl Out<'_> {
    fn push(&mut self, s: usize, e: usize, t: Tok) {
        if s >= e || e > self.code.len() || s < self.last {
            return;
        }
        if !self.code.is_char_boundary(s) || !self.code.is_char_boundary(e) {
            return;
        }
        self.v.push((s..e, t));
        self.last = e;
    }
}

#[inline]
fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}
fn is_id_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}
fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}
fn clen(c: u8) -> usize {
    if c < 0x80 {
        1
    } else if c < 0xE0 {
        2
    } else if c < 0xF0 {
        3
    } else {
        4
    }
}
/// Index of the next `\n` at or after `i`, or the length.
fn eol(b: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < b.len() && b[j] != b'\n' {
        j += 1;
    }
    j
}
fn trim_cr(b: &[u8], s: usize, e: usize) -> usize {
    if e > s && b[e - 1] == b'\r' {
        e - 1
    } else {
        e
    }
}
/// Index just past the first `pat` at or after `i`, or the length.
fn find_close(b: &[u8], i: usize, pat: &[u8]) -> usize {
    let n = b.len();
    let mut j = i;
    while j + pat.len() <= n {
        if &b[j..j + pat.len()] == pat {
            return j + pat.len();
        }
        j += 1;
    }
    n
}
fn at_line_start(b: &[u8], i: usize) -> bool {
    let mut j = i;
    while j > 0 && (b[j - 1] == b' ' || b[j - 1] == b'\t') {
        j -= 1;
    }
    j == 0 || b[j - 1] == b'\n'
}
/// Scan a quoted string starting at the opening quote `i`; returns the end (exclusive).
/// Single-line strings stop before a newline.
fn scan_quoted(b: &[u8], i: usize, q: u8, multiline: bool) -> usize {
    let n = b.len();
    let mut j = i + 1;
    while j < n {
        let c = b[j];
        if c == b'\\' {
            j += 2;
            continue;
        }
        if c == q {
            return j + 1;
        }
        if c == b'\n' && !multiline {
            return j;
        }
        j += 1;
    }
    n
}
/// Number with optional radix prefix, underscores, exponent and suffix letters.
fn scan_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i;
    let mut hex = false;
    if b[j] == b'0' && matches!(at(b, j + 1), b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
        hex = true;
        j += 2;
    }
    let mut dot = false;
    while j < n {
        let c = b[j];
        if c.is_ascii_alphanumeric() || c == b'_' {
            if !hex && (c == b'e' || c == b'E') && matches!(at(b, j + 1), b'+' | b'-') && at(b, j + 2).is_ascii_digit() {
                j += 2;
            }
            j += 1;
        } else if c == b'.' && !dot && !hex && at(b, j + 1).is_ascii_digit() {
            dot = true;
            j += 1;
        } else {
            break;
        }
    }
    j
}
fn has(lists: &[&[&str]], w: &str) -> bool {
    lists.iter().any(|l| l.contains(&w))
}
fn has_ci(list: &[&str], w: &str) -> bool {
    list.iter().any(|k| k.eq_ignore_ascii_case(w))
}
/// Is `w` an ALL_CAPS style name (at least two chars, upper/digit/underscore)?
fn all_caps(w: &str) -> bool {
    w.len() > 1 && w.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}
fn next_is_call(b: &[u8], mut j: usize) -> bool {
    while at(b, j) == b' ' || at(b, j) == b'\t' {
        j += 1;
    }
    at(b, j) == b'('
}

// ---------------------------------------------------------------------------------------------
// C family (and Lua, Ruby, Perl via flags)
// ---------------------------------------------------------------------------------------------

struct Lang {
    kw: &'static [&'static [&'static str]],
    ty: &'static [&'static str],
    cons: &'static [&'static str],
    slash: bool,
    hash: bool,
    dash: bool,
    block: bool,
    pp: bool,
    at_meta: bool,
    dollar_var: bool,
    at_var: bool,
    sym: bool,
    js: bool,
    tmpl: bool,
    rust: bool,
    php: bool,
    lua: bool,
    triple: bool,
    go_raw: bool,
    ml: bool,
}

const BASE: Lang = Lang {
    kw: &[],
    ty: &[],
    cons: &["true", "false", "null"],
    slash: true,
    hash: false,
    dash: false,
    block: true,
    pp: false,
    at_meta: false,
    dollar_var: false,
    at_var: false,
    sym: false,
    js: false,
    tmpl: false,
    rust: false,
    php: false,
    lua: false,
    triple: false,
    go_raw: false,
    ml: false,
};

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let",
    "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "type", "unsafe",
    "use", "where", "while", "union",
];
const RUST_TY: &[&str] =
    &["i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool", "char", "str"];
const C_KW: &[&str] = &[
    "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern", "for", "goto", "if", "inline", "register",
    "restrict", "return", "sizeof", "static", "struct", "switch", "typedef", "union", "volatile", "while",
];
const C_TY: &[&str] = &[
    "char",
    "int",
    "long",
    "short",
    "float",
    "double",
    "void",
    "unsigned",
    "signed",
    "bool",
    "_Bool",
    "size_t",
    "ssize_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "ptrdiff_t",
    "intptr_t",
    "uintptr_t",
    "wchar_t",
    "char16_t",
    "char32_t",
    "char8_t",
];
const CPP_KW: &[&str] = &[
    "class",
    "namespace",
    "template",
    "typename",
    "public",
    "private",
    "protected",
    "virtual",
    "override",
    "final",
    "new",
    "delete",
    "this",
    "try",
    "catch",
    "throw",
    "using",
    "operator",
    "constexpr",
    "consteval",
    "noexcept",
    "explicit",
    "friend",
    "mutable",
    "static_cast",
    "dynamic_cast",
    "reinterpret_cast",
    "const_cast",
    "decltype",
    "co_await",
    "co_return",
    "co_yield",
    "concept",
    "requires",
    "nullptr",
    "export",
    "import",
    "module",
    "thread_local",
    "static_assert",
    "alignas",
    "alignof",
];
const CS_KW: &[&str] = &[
    "abstract",
    "as",
    "base",
    "break",
    "case",
    "catch",
    "checked",
    "class",
    "const",
    "continue",
    "default",
    "delegate",
    "do",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "finally",
    "fixed",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "interface",
    "internal",
    "is",
    "lock",
    "namespace",
    "new",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sealed",
    "sizeof",
    "stackalloc",
    "static",
    "struct",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "unchecked",
    "unsafe",
    "using",
    "virtual",
    "volatile",
    "while",
    "var",
    "async",
    "await",
    "record",
    "init",
    "get",
    "set",
    "yield",
    "where",
    "when",
    "with",
    "required",
    "partial",
];
const CS_TY: &[&str] = &[
    "bool", "byte", "char", "decimal", "double", "float", "int", "long", "object", "sbyte", "short", "string", "uint", "ulong", "ushort",
    "void", "dynamic",
];
const JAVA_KW: &[&str] = &[
    "abstract",
    "assert",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "interface",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "volatile",
    "while",
    "var",
    "record",
    "sealed",
    "permits",
    "yield",
];
const JAVA_TY: &[&str] = &["boolean", "byte", "char", "double", "float", "int", "long", "short", "void"];
const KOTLIN_KW: &[&str] = &[
    "as",
    "break",
    "class",
    "continue",
    "do",
    "else",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "object",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "try",
    "typealias",
    "val",
    "var",
    "when",
    "while",
    "by",
    "catch",
    "constructor",
    "delegate",
    "finally",
    "get",
    "import",
    "init",
    "set",
    "value",
    "where",
    "abstract",
    "annotation",
    "companion",
    "const",
    "data",
    "enum",
    "final",
    "inner",
    "internal",
    "lateinit",
    "open",
    "operator",
    "out",
    "override",
    "private",
    "protected",
    "public",
    "sealed",
    "suspend",
    "vararg",
];
const SWIFT_KW: &[&str] = &[
    "associatedtype",
    "class",
    "deinit",
    "enum",
    "extension",
    "fileprivate",
    "func",
    "import",
    "init",
    "inout",
    "internal",
    "let",
    "open",
    "operator",
    "private",
    "protocol",
    "public",
    "rethrows",
    "static",
    "struct",
    "subscript",
    "typealias",
    "var",
    "break",
    "case",
    "catch",
    "continue",
    "default",
    "defer",
    "do",
    "else",
    "fallthrough",
    "for",
    "guard",
    "if",
    "in",
    "repeat",
    "return",
    "throw",
    "switch",
    "where",
    "while",
    "as",
    "await",
    "async",
    "is",
    "super",
    "self",
    "Self",
    "throws",
    "try",
    "some",
    "any",
    "actor",
    "lazy",
    "override",
    "final",
    "mutating",
    "weak",
    "nonisolated",
];
const GO_KW: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];
const GO_TY: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "any",
];
const JS_KW: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "get",
    "set",
    "as",
];
const TS_KW: &[&str] = &[
    "abstract",
    "declare",
    "enum",
    "implements",
    "interface",
    "keyof",
    "namespace",
    "private",
    "protected",
    "public",
    "readonly",
    "type",
    "is",
    "infer",
    "satisfies",
    "override",
    "module",
    "unique",
    "asserts",
];
const TS_TY: &[&str] = &["string", "number", "boolean", "any", "unknown", "never", "object", "symbol", "bigint"];
const JS_CONS: &[&str] = &["true", "false", "null", "undefined", "NaN", "Infinity"];
const DART_KW: &[&str] = &[
    "abstract",
    "as",
    "assert",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "covariant",
    "default",
    "deferred",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "extension",
    "external",
    "factory",
    "final",
    "finally",
    "for",
    "get",
    "hide",
    "if",
    "implements",
    "import",
    "in",
    "interface",
    "is",
    "late",
    "library",
    "mixin",
    "new",
    "on",
    "operator",
    "part",
    "required",
    "rethrow",
    "return",
    "set",
    "show",
    "static",
    "super",
    "switch",
    "sync",
    "this",
    "throw",
    "try",
    "typedef",
    "var",
    "while",
    "with",
    "yield",
];
const DART_TY: &[&str] = &["int", "double", "num", "bool", "void", "dynamic"];
const SCALA_KW: &[&str] = &[
    "abstract",
    "case",
    "catch",
    "class",
    "def",
    "do",
    "else",
    "extends",
    "final",
    "finally",
    "for",
    "forSome",
    "if",
    "implicit",
    "import",
    "lazy",
    "match",
    "new",
    "object",
    "override",
    "package",
    "private",
    "protected",
    "return",
    "sealed",
    "super",
    "this",
    "throw",
    "trait",
    "try",
    "type",
    "val",
    "var",
    "while",
    "with",
    "yield",
    "given",
    "using",
    "enum",
    "then",
];
const PHP_KW: &[&str] = &[
    "abstract",
    "and",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "goto",
    "if",
    "implements",
    "include",
    "include_once",
    "instanceof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
    "enum",
];
const ZIG_KW: &[&str] = &[
    "align",
    "allowzero",
    "and",
    "anyframe",
    "anytype",
    "asm",
    "async",
    "await",
    "break",
    "callconv",
    "catch",
    "comptime",
    "const",
    "continue",
    "defer",
    "else",
    "enum",
    "errdefer",
    "error",
    "export",
    "extern",
    "fn",
    "for",
    "if",
    "inline",
    "noalias",
    "nosuspend",
    "noinline",
    "opaque",
    "or",
    "orelse",
    "packed",
    "pub",
    "resume",
    "return",
    "linksection",
    "struct",
    "suspend",
    "switch",
    "test",
    "threadlocal",
    "try",
    "union",
    "unreachable",
    "usingnamespace",
    "var",
    "volatile",
    "while",
];
const ZIG_TY: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f16", "f32", "f64", "f128", "bool", "void",
    "anyerror", "type", "noreturn", "c_int", "c_uint", "c_long", "c_char",
];
const LUA_KW: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in", "local", "not", "or", "repeat", "return", "then",
    "until", "while",
];
const RUBY_KW: &[&str] = &[
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "defined?",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "for",
    "if",
    "in",
    "module",
    "next",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
    "puts",
    "require",
    "require_relative",
    "attr_accessor",
    "attr_reader",
    "attr_writer",
    "include",
    "extend",
    "raise",
    "lambda",
    "proc",
    "private",
    "public",
    "protected",
];
const PERL_KW: &[&str] = &[
    "my", "our", "local", "sub", "if", "elsif", "else", "unless", "while", "until", "for", "foreach", "return", "use", "no", "package",
    "require", "last", "next", "redo", "and", "or", "not", "eq", "ne", "lt", "gt", "le", "ge", "cmp", "print", "say", "defined", "undef",
    "ref", "bless", "die", "warn", "qw",
];
const FN_KW: &[&str] = &["fn", "func", "function", "fun", "def", "sub"];
const TY_KW: &[&str] =
    &["class", "struct", "enum", "trait", "interface", "type", "object", "record", "union", "protocol", "actor", "module"];

fn lang_cfg(name: &str) -> Option<Lang> {
    let l = match name {
        "rust" | "rs" => Lang { kw: &[RUST_KW], ty: RUST_TY, at_meta: false, rust: true, ..BASE },
        "c" | "h" => Lang { kw: &[C_KW], ty: C_TY, cons: &["NULL", "true", "false"], pp: true, ..BASE },
        "cpp" | "c++" | "cc" | "hpp" | "cxx" | "hxx" => {
            Lang { kw: &[C_KW, CPP_KW], ty: C_TY, cons: &["NULL", "true", "false"], pp: true, ..BASE }
        }
        "cs" | "csharp" | "c#" => Lang { kw: &[CS_KW], ty: CS_TY, pp: true, triple: true, at_meta: false, ..BASE },
        "java" => Lang { kw: &[JAVA_KW], ty: JAVA_TY, at_meta: true, triple: true, ..BASE },
        "kotlin" | "kt" => Lang { kw: &[KOTLIN_KW], at_meta: true, triple: true, ..BASE },
        "swift" => Lang { kw: &[SWIFT_KW], cons: &["true", "false", "nil"], at_meta: true, triple: true, ..BASE },
        "go" | "golang" => Lang { kw: &[GO_KW], ty: GO_TY, cons: &["true", "false", "nil", "iota"], go_raw: true, ..BASE },
        "javascript" | "js" | "jsx" | "mjs" | "cjs" => Lang { kw: &[JS_KW], cons: JS_CONS, js: true, tmpl: true, at_meta: true, ..BASE },
        "typescript" | "ts" | "tsx" | "mts" | "cts" => {
            Lang { kw: &[JS_KW, TS_KW], ty: TS_TY, cons: JS_CONS, js: true, tmpl: true, at_meta: true, ..BASE }
        }
        "dart" => Lang { kw: &[DART_KW], ty: DART_TY, at_meta: true, triple: true, ..BASE },
        "scala" => Lang { kw: &[SCALA_KW], at_meta: true, triple: true, ..BASE },
        "php" => Lang { kw: &[PHP_KW], hash: true, dollar_var: true, php: true, ..BASE },
        "zig" => Lang { kw: &[ZIG_KW], ty: ZIG_TY, cons: &["true", "false", "null", "undefined"], at_meta: true, block: false, ..BASE },
        "lua" => Lang { kw: &[LUA_KW], cons: &["true", "false", "nil"], slash: false, dash: true, block: false, lua: true, ..BASE },
        "ruby" | "rb" => Lang {
            kw: &[RUBY_KW],
            cons: &["true", "false", "nil"],
            slash: false,
            hash: true,
            block: false,
            at_var: true,
            dollar_var: true,
            sym: true,
            ..BASE
        },
        "perl" | "pl" => Lang { kw: &[PERL_KW], cons: &[], slash: false, hash: true, block: false, at_var: true, dollar_var: true, ..BASE },
        _ => return None,
    };
    Some(l)
}

#[derive(Clone, Copy, PartialEq)]
enum Decl {
    No,
    Fn,
    Ty,
}

fn clike(code: &str, l: &Lang, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut decl = Decl::No;
    let mut brace = 0usize;
    let mut tstack: Vec<usize> = Vec::new();
    while i < n {
        let c = b[i];
        let idst = is_id_start(c) || (l.js && c == b'$');
        if !c.is_ascii_whitespace() && !idst {
            decl = Decl::No;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let c1 = at(b, i + 1);
        // Comments.
        if l.slash && c == b'/' && c1 == b'/' {
            let e = eol(b, i);
            o.push(i, trim_cr(b, i, e), Tok::Comment);
            i = e;
            continue;
        }
        if l.block && c == b'/' && c1 == b'*' {
            let e = find_close(b, i + 2, b"*/");
            o.push(i, e, Tok::Comment);
            i = e;
            continue;
        }
        if l.dash && c == b'-' && c1 == b'-' {
            if l.lua && at(b, i + 2) == b'[' {
                if let Some(e) = lua_long(b, i + 2) {
                    o.push(i, e, Tok::Comment);
                    i = e;
                    continue;
                }
            }
            let e = eol(b, i);
            o.push(i, trim_cr(b, i, e), Tok::Comment);
            i = e;
            continue;
        }
        if c == b'#' {
            if (l.rust || l.php) && (c1 == b'[' || (l.rust && c1 == b'!' && at(b, i + 2) == b'[')) {
                let mut j = i + 1;
                let mut depth = 0;
                while j < n && b[j] != b'\n' {
                    if b[j] == b'[' {
                        depth += 1;
                    } else if b[j] == b']' {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    j += 1;
                }
                o.push(i, j, Tok::Meta);
                i = j;
                continue;
            }
            if l.hash {
                let e = eol(b, i);
                o.push(i, trim_cr(b, i, e), Tok::Comment);
                i = e;
                continue;
            }
            if l.pp && is_id_start(c1) && at_line_start(b, i) {
                let e = eol(b, i);
                o.push(i, trim_cr(b, i, e), Tok::Meta);
                i = e;
                continue;
            }
        }
        // Strings.
        if c == b'"' {
            let e = if l.triple && b[i..].starts_with(b"\"\"\"") {
                find_close(b, i + 3, b"\"\"\"")
            } else {
                scan_quoted(b, i, b'"', l.rust || l.ml)
            };
            o.push(i, e, Tok::String);
            i = e;
            continue;
        }
        if c == b'\'' {
            if l.rust {
                let j = i + 1;
                if at(b, j) == b'\\' {
                    let e = scan_quoted(b, i, b'\'', false);
                    o.push(i, e, Tok::String);
                    i = e;
                } else if j < n && at(b, j + clen(b[j])) == b'\'' {
                    let e = j + clen(b[j]) + 1;
                    o.push(i, e, Tok::String);
                    i = e;
                } else if is_id_start(at(b, j)) {
                    let mut k = j;
                    while k < n && is_id(b[k]) {
                        k += 1;
                    }
                    o.push(i, k, Tok::Meta);
                    i = k;
                } else {
                    i += 1;
                }
            } else {
                let e = scan_quoted(b, i, b'\'', false);
                o.push(i, e, Tok::String);
                i = e;
            }
            continue;
        }
        if c == b'`' {
            if l.tmpl {
                i = template(b, i, i + 1, &mut tstack, brace, o);
                continue;
            }
            if l.go_raw {
                let e = find_close(b, i + 1, b"`");
                o.push(i, e, Tok::String);
                i = e;
                continue;
            }
        }
        if l.tmpl {
            if c == b'{' {
                brace += 1;
            } else if c == b'}' {
                if tstack.last() == Some(&brace) {
                    tstack.pop();
                    i = template(b, i, i + 1, &mut tstack, brace, o);
                    continue;
                }
                brace = brace.saturating_sub(1);
            }
        }
        if l.lua && c == b'[' {
            if let Some(e) = lua_long(b, i) {
                o.push(i, e, Tok::String);
                i = e;
                continue;
            }
        }
        if l.php && c == b'<' && c1 == b'?' {
            let mut j = i + 2;
            while at(b, j).is_ascii_alphabetic() || at(b, j) == b'=' {
                j += 1;
            }
            o.push(i, j, Tok::Meta);
            i = j;
            continue;
        }
        if l.php && c == b'?' && c1 == b'>' {
            o.push(i, i + 2, Tok::Meta);
            i += 2;
            continue;
        }
        if c.is_ascii_digit() {
            let e = scan_number(b, i);
            o.push(i, e, Tok::Number);
            i = e;
            continue;
        }
        if l.dollar_var && c == b'$' && is_id_start(c1) {
            let mut j = i + 1;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            o.push(i, j, Tok::Variable);
            i = j;
            continue;
        }
        if c == b'@' {
            let mut j = i + 1;
            if l.at_var && at(b, j) == b'@' {
                j += 1;
            }
            if is_id_start(at(b, j)) && (l.at_var || l.at_meta) {
                while j < n && (is_id(b[j]) || (l.at_meta && !l.at_var && b[j] == b'.')) {
                    j += 1;
                }
                o.push(i, j, if l.at_var { Tok::Variable } else { Tok::Meta });
                i = j;
                continue;
            }
        }
        if l.sym && c == b':' {
            if c1 == b':' {
                i += 2;
                continue;
            }
            if is_id_start(c1) && i > 0 && !is_id(b[i - 1]) && b[i - 1] != b':' {
                let mut j = i + 1;
                while j < n && is_id(b[j]) {
                    j += 1;
                }
                o.push(i, j, Tok::Constant);
                i = j;
                continue;
            }
        }
        if idst {
            if l.rust && (c == b'r' || (c == b'b' && c1 == b'r')) {
                let mut j = i + if c == b'b' { 2 } else { 1 };
                let h0 = j;
                while at(b, j) == b'#' {
                    j += 1;
                }
                if at(b, j) == b'"' {
                    let mut pat = vec![b'"'];
                    pat.extend(std::iter::repeat(b'#').take(j - h0));
                    let e = find_close(b, j + 1, &pat);
                    o.push(i, e, Tok::String);
                    i = e;
                    continue;
                }
            }
            let mut j = i;
            while j < n && (is_id(b[j]) || (l.js && b[j] == b'$')) {
                j += 1;
            }
            let w = code.get(i..j).unwrap_or("");
            let cur = decl;
            decl = Decl::No;
            let tok = if has(l.kw, w) {
                if FN_KW.contains(&w) {
                    decl = Decl::Fn;
                } else if TY_KW.contains(&w) {
                    decl = Decl::Ty;
                }
                Some(Tok::Keyword)
            } else if l.cons.contains(&w) {
                Some(Tok::Constant)
            } else if l.ty.contains(&w) {
                Some(Tok::Type)
            } else if cur == Decl::Fn {
                Some(Tok::Function)
            } else if cur == Decl::Ty {
                Some(Tok::Type)
            } else if l.rust && at(b, j) == b'!' && at(b, j + 1) != b'=' {
                Some(Tok::Function)
            } else if next_is_call(b, j) {
                Some(Tok::Function)
            } else if c.is_ascii_uppercase() {
                Some(if all_caps(w) { Tok::Constant } else { Tok::Type })
            } else {
                None
            };
            if let Some(t) = tok {
                o.push(i, j, t);
            }
            i = j;
            continue;
        }
        i += 1;
    }
}

/// Scan a template literal segment. `start` is where scanning begins (after the opening backtick or
/// the closing brace of an interpolation); `tok_start` is where the String token begins.
fn template(b: &[u8], tok_start: usize, start: usize, stack: &mut Vec<usize>, brace: usize, o: &mut Out) -> usize {
    let n = b.len();
    let mut j = start;
    let mut end = n;
    while j < n {
        match b[j] {
            b'\\' => j += 2,
            b'`' => {
                end = j + 1;
                break;
            }
            b'$' if at(b, j + 1) == b'{' => {
                end = j + 2;
                stack.push(brace);
                break;
            }
            _ => j += 1,
        }
    }
    o.push(tok_start, end, Tok::String);
    end
}

/// If `b[i..]` opens a Lua long bracket (`[[`, `[==[`), return the index past its close (or the end).
fn lua_long(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    let mut level = 0;
    while at(b, j) == b'=' {
        level += 1;
        j += 1;
    }
    if at(b, j) != b'[' {
        return None;
    }
    let mut pat = vec![b']'];
    pat.extend(std::iter::repeat(b'=').take(level));
    pat.push(b']');
    Some(find_close(b, j + 1, &pat))
}

// ---------------------------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------------------------

const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for",
    "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
    "yield",
];
const PY_TY: &[&str] = &["int", "str", "float", "bool", "list", "dict", "set", "tuple", "bytes", "object", "type", "complex", "frozenset"];

fn py_string_end(b: &[u8], q: usize) -> usize {
    let c = b[q];
    if at(b, q + 1) == c && at(b, q + 2) == c {
        let n = b.len();
        let mut j = q + 3;
        while j < n {
            if b[j] == b'\\' {
                j += 2;
            } else if b[j] == c && at(b, j + 1) == c && at(b, j + 2) == c {
                return j + 3;
            } else {
                j += 1;
            }
        }
        n
    } else {
        scan_quoted(b, q, c, false)
    }
}

fn python(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut decl = Decl::No;
    while i < n {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if !is_id_start(c) {
            decl = Decl::No;
        }
        if c == b'#' {
            let e = eol(b, i);
            o.push(i, trim_cr(b, i, e), Tok::Comment);
            i = e;
        } else if c == b'"' || c == b'\'' {
            let e = py_string_end(b, i);
            o.push(i, e, Tok::String);
            i = e;
        } else if c == b'@' && at_line_start(b, i) {
            let mut j = i + 1;
            while j < n && (is_id(b[j]) || b[j] == b'.') {
                j += 1;
            }
            o.push(i, j, Tok::Meta);
            i = j;
        } else if c.is_ascii_digit() {
            let e = scan_number(b, i);
            o.push(i, e, Tok::Number);
            i = e;
        } else if is_id_start(c) {
            let mut j = i;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            let w = code.get(i..j).unwrap_or("");
            if j - i <= 3
                && matches!(at(b, j), b'"' | b'\'')
                && w.bytes().all(|x| matches!(x, b'r' | b'b' | b'f' | b'u' | b'R' | b'B' | b'F' | b'U'))
            {
                let e = py_string_end(b, j);
                o.push(i, e, Tok::String);
                i = e;
                continue;
            }
            let cur = decl;
            decl = Decl::No;
            let tok = if PY_KW.contains(&w) {
                if w == "def" {
                    decl = Decl::Fn;
                } else if w == "class" {
                    decl = Decl::Ty;
                }
                Some(Tok::Keyword)
            } else if matches!(w, "True" | "False" | "None") {
                Some(Tok::Constant)
            } else if matches!(w, "self" | "cls") {
                Some(Tok::Variable)
            } else if cur == Decl::Fn {
                Some(Tok::Function)
            } else if cur == Decl::Ty || PY_TY.contains(&w) {
                Some(Tok::Type)
            } else if c.is_ascii_uppercase() {
                Some(if all_caps(w) { Tok::Constant } else { Tok::Type })
            } else if next_is_call(b, j) {
                Some(Tok::Function)
            } else {
                None
            };
            if let Some(t) = tok {
                o.push(i, j, t);
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shell
// ---------------------------------------------------------------------------------------------

const SH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "select", "export",
    "local", "readonly", "declare", "unset", "return", "exit", "break", "continue", "source", "alias", "set", "shift", "trap", "eval",
    "exec", "time",
];

fn sh_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'%' | b'+' | b'~' | b',') || c >= 0x80
}

fn shell(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut decl = Decl::No;
    while i < n {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if !sh_word(c) {
            decl = Decl::No;
        }
        let prev = if i == 0 { b'\n' } else { b[i - 1] };
        let wsb = prev.is_ascii_whitespace() || matches!(prev, b';' | b'(' | b'&' | b'|' | b'{');
        match c {
            b'#' if wsb => {
                let e = eol(b, i);
                o.push(i, trim_cr(b, i, e), Tok::Comment);
                i = e;
            }
            b'"' => {
                let e = scan_quoted(b, i, b'"', true);
                o.push(i, e, Tok::String);
                i = e;
            }
            b'\'' => {
                let e = find_close(b, i + 1, b"'");
                o.push(i, e, Tok::String);
                i = e;
            }
            b'$' => {
                let c1 = at(b, i + 1);
                if c1 == b'{' {
                    let mut j = i + 2;
                    while j < n && b[j] != b'}' && b[j] != b'\n' {
                        j += 1;
                    }
                    if at(b, j) == b'}' {
                        j += 1;
                    }
                    o.push(i, j, Tok::Variable);
                    i = j;
                } else if is_id_start(c1) || c1.is_ascii_digit() {
                    let mut j = i + 1;
                    if c1.is_ascii_digit() {
                        j += 1;
                    } else {
                        while j < n && is_id(b[j]) {
                            j += 1;
                        }
                    }
                    o.push(i, j, Tok::Variable);
                    i = j;
                } else if matches!(c1, b'@' | b'?' | b'#' | b'!' | b'*' | b'$' | b'-') {
                    o.push(i, i + 2, Tok::Variable);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            b'\\' => {
                i += 1;
                if i < n {
                    i += clen(b[i]);
                }
                i = i.min(n);
            }
            b'-' if wsb && (at(b, i + 1).is_ascii_alphabetic() || (at(b, i + 1) == b'-' && at(b, i + 2).is_ascii_alphabetic())) => {
                let mut j = i + 1;
                while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b'_') {
                    j += 1;
                }
                o.push(i, j, Tok::Property);
                i = j;
            }
            _ if sh_word(c) => {
                let mut j = i;
                while j < n && sh_word(b[j]) {
                    j += 1;
                }
                let w = code.get(i..j).unwrap_or("");
                let cur = decl;
                decl = Decl::No;
                let ident = is_id_start(c) && w.bytes().all(is_id);
                let tok = if ident && at(b, j) == b'=' {
                    Some(Tok::Variable)
                } else if w.bytes().all(|x| x.is_ascii_digit()) {
                    Some(Tok::Number)
                } else if SH_KW.contains(&w) {
                    if w == "function" {
                        decl = Decl::Fn;
                    }
                    Some(Tok::Keyword)
                } else if cur == Decl::Fn || (at(b, j) == b'(' && at(b, j + 1) == b')') {
                    Some(Tok::Function)
                } else {
                    None
                };
                if let Some(t) = tok {
                    o.push(i, j, t);
                }
                i = j;
            }
            _ => i += 1,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// PowerShell
// ---------------------------------------------------------------------------------------------

const PS_KW: &[&str] = &[
    "begin",
    "break",
    "catch",
    "class",
    "continue",
    "data",
    "do",
    "dynamicparam",
    "else",
    "elseif",
    "end",
    "exit",
    "filter",
    "finally",
    "for",
    "foreach",
    "from",
    "function",
    "if",
    "in",
    "param",
    "process",
    "return",
    "switch",
    "throw",
    "trap",
    "try",
    "until",
    "using",
    "var",
    "while",
    "workflow",
    "parallel",
];
const PS_OPS: &[&str] = &[
    "eq",
    "ne",
    "gt",
    "ge",
    "lt",
    "le",
    "like",
    "notlike",
    "match",
    "notmatch",
    "contains",
    "notcontains",
    "in",
    "notin",
    "replace",
    "split",
    "join",
    "and",
    "or",
    "not",
    "xor",
    "is",
    "isnot",
    "as",
    "band",
    "bor",
    "bxor",
    "bnot",
    "shl",
    "shr",
    "ceq",
    "cne",
    "cgt",
    "clt",
    "cle",
    "cge",
    "ieq",
    "ine",
    "igt",
    "ilt",
    "ile",
    "ige",
    "ilike",
    "imatch",
    "f",
];

fn powershell(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut decl = Decl::No;
    while i < n {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if !is_id_start(c) {
            decl = Decl::No;
        }
        let c1 = at(b, i + 1);
        let prev = if i == 0 { b'\n' } else { b[i - 1] };
        if c == b'<' && c1 == b'#' {
            let e = find_close(b, i + 2, b"#>");
            o.push(i, e, Tok::Comment);
            i = e;
        } else if c == b'#' {
            let e = eol(b, i);
            o.push(i, trim_cr(b, i, e), Tok::Comment);
            i = e;
        } else if c == b'@' && (c1 == b'"' || c1 == b'\'') {
            let e = find_close(b, i + 2, if c1 == b'"' { b"\"@" } else { b"'@" });
            o.push(i, e, Tok::String);
            i = e;
        } else if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < n {
                if c == b'"' && b[j] == b'`' {
                    j += 2;
                } else if b[j] == c {
                    if at(b, j + 1) == c {
                        j += 2;
                    } else {
                        j += 1;
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            let e = j.min(n);
            o.push(i, e, Tok::String);
            i = e;
        } else if c == b'$' || (c == b'@' && is_id_start(c1)) {
            let mut j = i + 1;
            if c == b'$' && c1 == b'{' {
                while j < n && b[j] != b'}' && b[j] != b'\n' {
                    j += 1;
                }
                if at(b, j) == b'}' {
                    j += 1;
                }
            } else if is_id(c1) {
                while j < n && (is_id(b[j]) || (b[j] == b':' && is_id_start(at(b, j + 1)))) {
                    j += 1;
                }
            } else if matches!(c1, b'?' | b'$' | b'^') {
                j += 1;
            }
            let w = code.get(i..j).unwrap_or("");
            let t = if w.eq_ignore_ascii_case("$true") || w.eq_ignore_ascii_case("$false") || w.eq_ignore_ascii_case("$null") {
                Tok::Constant
            } else {
                Tok::Variable
            };
            if j > i + 1 {
                o.push(i, j, t);
            }
            i = j;
        } else if c == b'-' && is_id_start(c1) && (prev.is_ascii_whitespace() || prev == b'(') {
            let mut j = i + 1;
            while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            let w = code.get(i + 1..j).unwrap_or("");
            let t = if has_ci(PS_OPS, w) { Tok::Keyword } else { Tok::Property };
            o.push(i, j, t);
            i = j;
        } else if c == b'[' && is_id_start(c1) {
            let mut j = i + 1;
            while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'.') {
                j += 1;
            }
            if at(b, j) == b']' || at(b, j) == b'(' || (at(b, j) == b'[' && at(b, j + 1) == b']') {
                o.push(i + 1, j, Tok::Type);
            }
            i += 1;
        } else if c.is_ascii_digit() {
            let e = scan_number(b, i);
            o.push(i, e, Tok::Number);
            i = e;
        } else if is_id_start(c) {
            let mut j = i;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            let mut verb_noun = false;
            if c.is_ascii_uppercase() {
                while at(b, j) == b'-' && at(b, j + 1).is_ascii_uppercase() {
                    verb_noun = true;
                    j += 1;
                    while j < n && is_id(b[j]) {
                        j += 1;
                    }
                }
            }
            let w = code.get(i..j).unwrap_or("");
            let cur = decl;
            decl = Decl::No;
            let tok = if verb_noun {
                Some(Tok::Function)
            } else if has_ci(PS_KW, w) {
                if w.eq_ignore_ascii_case("function") || w.eq_ignore_ascii_case("filter") {
                    decl = Decl::Fn;
                }
                Some(Tok::Keyword)
            } else if cur == Decl::Fn {
                Some(Tok::Function)
            } else {
                None
            };
            if let Some(t) = tok {
                o.push(i, j, t);
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Data: JSON, YAML, TOML/INI
// ---------------------------------------------------------------------------------------------

fn json(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        let c = b[i];
        let c1 = at(b, i + 1);
        if c == b'/' && c1 == b'/' {
            let e = eol(b, i);
            o.push(i, trim_cr(b, i, e), Tok::Comment);
            i = e;
        } else if c == b'/' && c1 == b'*' {
            let e = find_close(b, i + 2, b"*/");
            o.push(i, e, Tok::Comment);
            i = e;
        } else if c == b'"' || c == b'\'' {
            let e = scan_quoted(b, i, c, false);
            let mut k = e;
            while k < n && b[k].is_ascii_whitespace() {
                k += 1;
            }
            o.push(i, e, if at(b, k) == b':' { Tok::Property } else { Tok::String });
            i = e;
        } else if c.is_ascii_digit() || ((c == b'-' || c == b'+') && (c1.is_ascii_digit() || (c1 == b'.' && at(b, i + 2).is_ascii_digit())))
        {
            let s = if c.is_ascii_digit() { i } else { i + 1 };
            let e = if at(b, s) == b'.' { scan_number(b, s + 1) } else { scan_number(b, s) };
            o.push(i, e, Tok::Number);
            i = e;
        } else if is_id_start(c) {
            let mut j = i;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            let w = code.get(i..j).unwrap_or("");
            let mut k = j;
            while k < n && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if matches!(w, "true" | "false" | "null" | "NaN" | "Infinity") {
                o.push(i, j, Tok::Constant);
            } else if at(b, k) == b':' {
                o.push(i, j, Tok::Property);
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

fn yaml(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut block: Option<usize> = None;
    while i < n {
        let ls = i;
        let le = eol(b, i);
        let nx = if le < n { le + 1 } else { n };
        let end = trim_cr(b, ls, le);
        i = nx;
        let mut p = ls;
        while p < end && (b[p] == b' ' || b[p] == b'\t') {
            p += 1;
        }
        let indent = p - ls;
        if let Some(par) = block {
            if p >= end {
                continue;
            }
            if indent > par {
                o.push(p, end, Tok::String);
                continue;
            }
            block = None;
        }
        if p >= end {
            continue;
        }
        if b[p] == b'#' {
            o.push(p, end, Tok::Comment);
            continue;
        }
        let mut q = p;
        if indent == 0 && (b[p..end].starts_with(b"---") || b[p..end].starts_with(b"...")) && (end - p == 3 || b[p + 3] == b' ') {
            o.push(p, p + 3, Tok::Meta);
            q = p + 3;
        }
        while q < end && b[q] == b'-' && (q + 1 >= end || b[q + 1] == b' ') {
            q += 1;
            while q < end && b[q] == b' ' {
                q += 1;
            }
        }
        // Key detection.
        let mut vstart = q;
        if q < end && !matches!(b[q], b'[' | b'{' | b'|' | b'>' | b'#' | b'&' | b'*' | b'!') {
            let mut found: Option<(usize, usize)> = None; // (key end, colon index)
            if b[q] == b'"' || b[q] == b'\'' {
                let e = scan_quoted(b, q, b[q], false).min(end);
                if at(b, e) == b':' && (e + 1 >= end || b[e + 1] == b' ') {
                    found = Some((e, e));
                }
            } else {
                let mut k = q;
                while k < end {
                    if b[k] == b':' && (k + 1 >= end || b[k + 1] == b' ') {
                        let mut ke = k;
                        while ke > q && b[ke - 1] == b' ' {
                            ke -= 1;
                        }
                        found = Some((ke, k));
                        break;
                    }
                    if b[k] == b'#' && k > q && b[k - 1] == b' ' {
                        break;
                    }
                    k += 1;
                }
            }
            if let Some((ke, colon)) = found {
                o.push(q, ke, Tok::Property);
                vstart = colon + 1;
            }
        }
        if yaml_value(b, vstart, end, o) {
            block = Some(indent);
        }
    }
}

/// Scan the value part of a YAML line. Returns true if it opens a block scalar.
fn yaml_value(b: &[u8], start: usize, end: usize, o: &mut Out) -> bool {
    let mut i = start;
    let mut first = true;
    while i < end {
        let c = b[i];
        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }
        let was_first = first;
        first = false;
        if c == b'#' && (i == start || b[i - 1] == b' ' || b[i - 1] == b'\t') {
            o.push(i, end, Tok::Comment);
            return false;
        }
        if c == b'"' || c == b'\'' {
            let e = scan_quoted(b, i, c, false).min(end);
            o.push(i, e, Tok::String);
            i = e.max(i + 1);
            continue;
        }
        if matches!(c, b'[' | b']' | b'{' | b'}' | b',') {
            i += 1;
            continue;
        }
        if was_first && (c == b'|' || c == b'>') {
            let mut j = i + 1;
            while j < end && (matches!(b[j], b'+' | b'-') || b[j].is_ascii_digit()) {
                j += 1;
            }
            let mut k = j;
            while k < end && b[k] == b' ' {
                k += 1;
            }
            if k >= end || b[k] == b'#' {
                o.push(i, j, Tok::Meta);
                return true;
            }
        }
        let mut j = i;
        while j < end && !matches!(b[j], b' ' | b'\t' | b',' | b'[' | b']' | b'{' | b'}') {
            j += 1;
        }
        if j == i {
            j = i + 1;
        }
        let w = &b[i..j];
        let ws = std::str::from_utf8(w).unwrap_or("");
        let t = if matches!(c, b'&' | b'*' | b'!') {
            Some(Tok::Meta)
        } else if matches!(ws.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~") {
            Some(Tok::Constant)
        } else if looks_numeric(w) {
            Some(Tok::Number)
        } else {
            None
        };
        if let Some(t) = t {
            o.push(i, j, t);
        }
        i = j;
    }
    false
}

fn looks_numeric(w: &[u8]) -> bool {
    let s = if matches!(at(w, 0), b'+' | b'-') { 1 } else { 0 };
    at(w, s).is_ascii_digit()
        && w[s..].iter().all(|c| c.is_ascii_hexdigit() || matches!(c, b'x' | b'X' | b'o' | b'_' | b'.' | b'+' | b'-' | b':' | b'T' | b'Z'))
}

fn ini(code: &str, o: &mut Out, toml: bool) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        let le = eol(b, i);
        let end = trim_cr(b, i, le);
        let nx = if le < n { le + 1 } else { n };
        let mut p = i;
        while p < end && (b[p] == b' ' || b[p] == b'\t') {
            p += 1;
        }
        if p >= end {
            i = nx;
            continue;
        }
        let c = b[p];
        if c == b'#' || (!toml && c == b';') {
            o.push(p, end, Tok::Comment);
            i = nx;
        } else if c == b'[' {
            let mut k = p;
            while k < end && b[k] != b']' {
                k += 1;
            }
            while k < end && b[k] == b']' {
                k += 1;
            }
            o.push(p, k, Tok::Tag);
            i = ini_value(b, k, toml, o);
        } else {
            let mut k = p;
            let mut sep = None;
            if b[k] == b'"' || b[k] == b'\'' {
                k = scan_quoted(b, k, b[k], false).min(end);
            }
            while k < end {
                if b[k] == b'=' || (!toml && b[k] == b':') {
                    sep = Some(k);
                    break;
                }
                k += 1;
            }
            let v = if let Some(s) = sep {
                let mut ke = s;
                while ke > p && (b[ke - 1] == b' ' || b[ke - 1] == b'\t') {
                    ke -= 1;
                }
                o.push(p, ke, Tok::Property);
                s + 1
            } else {
                p
            };
            i = ini_value(b, v, toml, o);
        }
    }
}

/// Scan a value to end of line (TOML triple-quoted strings may run on). Returns the next line start.
fn ini_value(b: &[u8], start: usize, toml: bool, o: &mut Out) -> usize {
    let n = b.len();
    let mut i = start;
    while i < n {
        let c = b[i];
        if c == b'\n' {
            return i + 1;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        let word_start = i == start || b[i - 1] == b' ' || b[i - 1] == b'\t';
        if c == b'#' || (!toml && c == b';' && word_start) {
            let e = eol(b, i);
            o.push(i, trim_cr(b, i, e), Tok::Comment);
            i = e;
        } else if toml && (b[i..].starts_with(b"\"\"\"") || b[i..].starts_with(b"'''")) {
            let e = find_close(b, i + 3, &b[i..i + 3]);
            o.push(i, e, Tok::String);
            i = e;
        } else if c == b'"' || c == b'\'' {
            let e = scan_quoted(b, i, c, false);
            o.push(i, e, Tok::String);
            i = e;
        } else if c.is_ascii_digit() || ((c == b'+' || c == b'-') && at(b, i + 1).is_ascii_digit()) {
            let mut j = i + 1;
            while j < n && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'_' | b'.' | b'-' | b'+' | b':')) {
                j += 1;
            }
            o.push(i, j, Tok::Number);
            i = j;
        } else if is_id_start(c) {
            let mut j = i;
            while j < n && (is_id(b[j]) || b[j] == b'.' || b[j] == b'-') {
                j += 1;
            }
            let w = std::str::from_utf8(&b[i..j]).unwrap_or("");
            let lw = w.to_ascii_lowercase();
            let bools = matches!(lw.as_str(), "true" | "false") || (!toml && matches!(lw.as_str(), "yes" | "no" | "on" | "off"));
            if bools {
                o.push(i, j, Tok::Constant);
            } else if toml && matches!(lw.as_str(), "inf" | "nan") {
                o.push(i, j, Tok::Number);
            }
            i = j;
        } else {
            i += clen(c).min(n - i);
        }
    }
    n
}

// ---------------------------------------------------------------------------------------------
// Markup: HTML / XML, CSS
// ---------------------------------------------------------------------------------------------

fn markup(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let c1 = at(b, i + 1);
        if b[i..].starts_with(b"<!--") {
            let e = find_close(b, i + 4, b"-->");
            o.push(i, e, Tok::Comment);
            i = e;
            continue;
        }
        if b[i..].starts_with(b"<![CDATA[") {
            let e = find_close(b, i + 9, b"]]>");
            o.push(i, e, Tok::String);
            i = e;
            continue;
        }
        if c1 == b'!' || c1 == b'?' {
            let e = if c1 == b'?' { find_close(b, i + 2, b"?>") } else { find_close(b, i + 2, b">") };
            o.push(i, e, Tok::Meta);
            i = e;
            continue;
        }
        let closing = c1 == b'/';
        let ns = i + 1 + closing as usize;
        if !at(b, ns).is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let mut j = ns;
        while j < n && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'-' | b'_' | b':' | b'.')) {
            j += 1;
        }
        o.push(i, j, Tok::Tag);
        let name = code[ns..j].to_ascii_lowercase();
        let mut k = j;
        let mut selfclose = false;
        let mut closed = false;
        while k < n {
            while k < n && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k >= n {
                break;
            }
            let c = b[k];
            if c == b'>' {
                o.push(k, k + 1, Tok::Tag);
                k += 1;
                closed = true;
                break;
            }
            if c == b'/' && at(b, k + 1) == b'>' {
                o.push(k, k + 2, Tok::Tag);
                k += 2;
                selfclose = true;
                break;
            }
            if c == b'<' {
                break;
            }
            if c == b'/' {
                k += 1;
                continue;
            }
            if c == b'"' || c == b'\'' {
                let e = find_close(b, k + 1, &[c]);
                o.push(k, e, Tok::String);
                k = e;
                continue;
            }
            let s = k;
            while k < n
                && !(b[k].is_ascii_whitespace() || matches!(b[k], b'=' | b'>' | b'<' | b'"' | b'\''))
                && !(b[k] == b'/' && at(b, k + 1) == b'>')
            {
                k += 1;
            }
            if k == s {
                k += 1;
                continue;
            }
            o.push(s, k, Tok::Property);
            let mut m = k;
            while m < n && b[m].is_ascii_whitespace() {
                m += 1;
            }
            if at(b, m) == b'=' {
                m += 1;
                while m < n && b[m].is_ascii_whitespace() {
                    m += 1;
                }
                let q = at(b, m);
                if q == b'"' || q == b'\'' {
                    let e = find_close(b, m + 1, &[q]);
                    o.push(m, e, Tok::String);
                    k = e;
                } else {
                    let s2 = m;
                    while m < n && !b[m].is_ascii_whitespace() && b[m] != b'>' {
                        m += 1;
                    }
                    o.push(s2, m, Tok::String);
                    k = m;
                }
            }
        }
        i = k;
        if closed && !closing && !selfclose && (name == "script" || name == "style") {
            let mut e = n;
            let mut p = k;
            while p + 2 + name.len() <= n {
                if b[p] == b'<' && b[p + 1] == b'/' && b[p + 2..p + 2 + name.len()].eq_ignore_ascii_case(name.as_bytes()) {
                    e = p;
                    break;
                }
                p += 1;
            }
            let sub = highlight(if name == "script" { "js" } else { "css" }, &code[k..e]);
            for (r, t) in sub {
                o.push(k + r.start, k + r.end, t);
            }
            i = e;
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Sel,
    Decl,
    Val,
}

fn css_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c >= 0x80
}

fn css_lookahead(b: &[u8], i: usize) -> Mode {
    let n = b.len();
    let mut j = i;
    while j < n {
        match b[j] {
            b'{' => return Mode::Sel,
            b';' | b'}' => return Mode::Decl,
            b'"' | b'\'' => j = scan_quoted(b, j, b[j], false).max(j + 1),
            b'/' if at(b, j + 1) == b'*' => j = find_close(b, j + 2, b"*/"),
            b'#' if at(b, j + 1) == b'{' => j = find_close(b, j + 2, b"}"),
            _ => j += 1,
        }
    }
    Mode::Decl
}

fn css(code: &str, o: &mut Out, pre: bool) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut stmt = true;
    let mut m = Mode::Val;
    while i < n {
        let c = b[i];
        let c1 = at(b, i + 1);
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && c1 == b'*' {
            let e = find_close(b, i + 2, b"*/");
            o.push(i, e, Tok::Comment);
            i = e;
            continue;
        }
        if pre && c == b'/' && c1 == b'/' && (i == 0 || b[i - 1] != b':') {
            let e = eol(b, i);
            o.push(i, trim_cr(b, i, e), Tok::Comment);
            i = e;
            continue;
        }
        if matches!(c, b'{' | b'}' | b';') {
            stmt = true;
            i += 1;
            continue;
        }
        if stmt {
            stmt = false;
            if c == b'@' {
                let mut j = i + 1;
                while j < n && css_ident(b[j]) {
                    j += 1;
                }
                let mut k = j;
                while at(b, k) == b' ' {
                    k += 1;
                }
                let t = if pre && at(b, k) == b':' { Tok::Variable } else { Tok::Keyword };
                o.push(i, j, t);
                m = Mode::Val;
                i = j.max(i + 1);
                continue;
            }
            m = css_lookahead(b, i);
        }
        match m {
            Mode::Sel => {
                if c == b'"' || c == b'\'' {
                    let e = scan_quoted(b, i, c, false);
                    o.push(i, e, Tok::String);
                    i = e;
                } else if c == b'[' {
                    i += 1;
                    let mut first = true;
                    while i < n && b[i] != b']' && b[i] != b'\n' {
                        if b[i] == b'"' || b[i] == b'\'' {
                            let e = scan_quoted(b, i, b[i], false);
                            o.push(i, e, Tok::String);
                            i = e;
                        } else if first && css_ident(b[i]) {
                            let s = i;
                            while i < n && css_ident(b[i]) {
                                i += 1;
                            }
                            o.push(s, i, Tok::Property);
                            first = false;
                        } else {
                            i += 1;
                        }
                    }
                } else if matches!(c, b'.' | b'#' | b':' | b'&' | b'%') || css_ident(c) || c == b'*' {
                    let mut j = i;
                    if !css_ident(c) {
                        j += 1;
                        if c == b':' && c1 == b':' {
                            j += 1;
                        }
                    }
                    let id = j;
                    while j < n && css_ident(b[j]) {
                        j += 1;
                    }
                    if j > id || c == b'*' || c == b'&' {
                        o.push(i, j, Tok::Tag);
                    }
                    i = j.max(i + 1);
                } else {
                    i += 1;
                }
            }
            Mode::Decl => {
                if c.is_ascii_alphabetic() || matches!(c, b'-' | b'_' | b'$' | b'@' | b'*') {
                    let mut j = i + 1;
                    while j < n && css_ident(b[j]) {
                        j += 1;
                    }
                    let mut k = j;
                    while at(b, k) == b' ' {
                        k += 1;
                    }
                    if at(b, k) == b':' {
                        o.push(i, j, if c == b'$' || c == b'@' { Tok::Variable } else { Tok::Property });
                        i = j;
                    }
                }
                m = Mode::Val;
            }
            Mode::Val => {
                let n1 = at(b, i + 1);
                if c == b'"' || c == b'\'' {
                    let e = scan_quoted(b, i, c, false);
                    o.push(i, e, Tok::String);
                    i = e;
                } else if c == b'#' {
                    let mut j = i + 1;
                    while at(b, j).is_ascii_hexdigit() {
                        j += 1;
                    }
                    if matches!(j - i - 1, 3 | 4 | 6 | 8) && !css_ident(at(b, j)) {
                        o.push(i, j, Tok::Constant);
                    }
                    i = j.max(i + 1);
                } else if c.is_ascii_digit()
                    || (matches!(c, b'.' | b'-' | b'+') && (n1.is_ascii_digit() || (n1 == b'.' && at(b, i + 2).is_ascii_digit())))
                {
                    let mut j = i;
                    if matches!(b[j], b'-' | b'+') {
                        j += 1;
                    }
                    while at(b, j).is_ascii_digit() {
                        j += 1;
                    }
                    if at(b, j) == b'.' && at(b, j + 1).is_ascii_digit() {
                        j += 1;
                        while at(b, j).is_ascii_digit() {
                            j += 1;
                        }
                    }
                    while at(b, j).is_ascii_alphabetic() || at(b, j) == b'%' {
                        j += 1;
                    }
                    o.push(i, j, Tok::Number);
                    i = j.max(i + 1);
                } else if c == b'!' && n1.is_ascii_alphabetic() {
                    let mut j = i + 1;
                    while css_ident(at(b, j)) {
                        j += 1;
                    }
                    o.push(i, j, Tok::Keyword);
                    i = j;
                } else if (c == b'$' || c == b'@') && is_id_start(n1) {
                    let mut j = i + 1;
                    while css_ident(at(b, j)) {
                        j += 1;
                    }
                    o.push(i, j, Tok::Variable);
                    i = j;
                } else if c.is_ascii_alphabetic() || c == b'_' || (c == b'-' && (n1.is_ascii_alphabetic() || n1 == b'-' || n1 == b'_')) {
                    let mut j = i + 1;
                    while css_ident(at(b, j)) {
                        j += 1;
                    }
                    if at(b, j) == b'(' {
                        o.push(i, j, Tok::Function);
                    }
                    i = j;
                } else {
                    i += 1;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// SQL
// ---------------------------------------------------------------------------------------------

const SQL_KW: &[&str] = &[
    "select",
    "from",
    "where",
    "and",
    "or",
    "not",
    "in",
    "is",
    "like",
    "ilike",
    "between",
    "join",
    "inner",
    "left",
    "right",
    "outer",
    "full",
    "cross",
    "on",
    "as",
    "group",
    "by",
    "order",
    "having",
    "limit",
    "offset",
    "union",
    "all",
    "distinct",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "alter",
    "drop",
    "index",
    "view",
    "database",
    "schema",
    "primary",
    "key",
    "foreign",
    "references",
    "unique",
    "default",
    "check",
    "constraint",
    "add",
    "column",
    "if",
    "exists",
    "case",
    "when",
    "then",
    "else",
    "end",
    "begin",
    "commit",
    "rollback",
    "transaction",
    "with",
    "recursive",
    "returning",
    "grant",
    "revoke",
    "truncate",
    "asc",
    "desc",
    "using",
    "natural",
    "over",
    "partition",
    "window",
    "explain",
    "analyze",
    "pragma",
    "vacuum",
    "trigger",
    "procedure",
    "function",
    "returns",
    "return",
    "declare",
    "cascade",
    "restrict",
    "top",
    "fetch",
    "next",
    "only",
    "rows",
    "row",
    "first",
    "last",
    "nulls",
    "intersect",
    "except",
    "rename",
    "to",
    "temporary",
    "temp",
    "replace",
    "unlock",
    "lock",
];
const SQL_TY: &[&str] = &[
    "int",
    "integer",
    "bigint",
    "smallint",
    "tinyint",
    "serial",
    "bigserial",
    "varchar",
    "char",
    "text",
    "boolean",
    "bool",
    "date",
    "time",
    "timestamp",
    "timestamptz",
    "datetime",
    "decimal",
    "numeric",
    "float",
    "double",
    "real",
    "blob",
    "json",
    "jsonb",
    "uuid",
    "bytea",
    "money",
    "interval",
];

fn sql(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        let c = b[i];
        let c1 = at(b, i + 1);
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'-' && c1 == b'-' {
            let e = eol(b, i);
            o.push(i, trim_cr(b, i, e), Tok::Comment);
            i = e;
        } else if c == b'/' && c1 == b'*' {
            let e = find_close(b, i + 2, b"*/");
            o.push(i, e, Tok::Comment);
            i = e;
        } else if c == b'\'' {
            let mut j = i + 1;
            while j < n {
                if b[j] == b'\'' {
                    if at(b, j + 1) == b'\'' {
                        j += 2;
                        continue;
                    }
                    j += 1;
                    break;
                }
                j += 1;
            }
            let e = j.min(n);
            o.push(i, e, Tok::String);
            i = e;
        } else if c == b'"' || c == b'`' {
            let mut j = i + 1;
            while j < n && b[j] != c && b[j] != b'\n' {
                j += 1;
            }
            if at(b, j) == c {
                j += 1;
            }
            o.push(i, j, Tok::Variable);
            i = j;
        } else if c.is_ascii_digit() {
            let e = scan_number(b, i);
            o.push(i, e, Tok::Number);
            i = e;
        } else if c == b':' && c1 == b':' {
            i += 2;
        } else if (c == b'@' || c == b':' || c == b'$') && (is_id_start(c1) || c1.is_ascii_digit()) {
            let mut j = i + 1;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            o.push(i, j, Tok::Variable);
            i = j;
        } else if is_id_start(c) {
            let mut j = i;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            let w = code.get(i..j).unwrap_or("").to_ascii_lowercase();
            let t = if SQL_KW.contains(&w.as_str()) {
                Some(Tok::Keyword)
            } else if SQL_TY.contains(&w.as_str()) {
                Some(Tok::Type)
            } else if matches!(w.as_str(), "null" | "true" | "false") {
                Some(Tok::Constant)
            } else if next_is_call(b, j) {
                Some(Tok::Function)
            } else {
                None
            };
            if let Some(t) = t {
                o.push(i, j, t);
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Diff, Dockerfile, Makefile, Markdown
// ---------------------------------------------------------------------------------------------

fn diff(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut in_hunk = false;
    while i < n {
        let le = eol(b, i);
        let end = trim_cr(b, i, le);
        let nx = if le < n { le + 1 } else { n };
        let line = &b[i..end];
        let tok = if line.starts_with(b"diff ") {
            in_hunk = false;
            Some(Tok::Meta)
        } else if line.starts_with(b"@@") {
            in_hunk = true;
            Some(Tok::Meta)
        } else if line.starts_with(b"--- ") && (!in_hunk || b[nx..].starts_with(b"+++ ")) {
            in_hunk = false;
            Some(Tok::Meta)
        } else if line.starts_with(b"+++ ") && !in_hunk {
            Some(Tok::Meta)
        } else if !in_hunk
            && [&b"index "[..], b"new file", b"deleted file", b"similarity ", b"rename ", b"old mode", b"new mode", b"Binary files"]
                .iter()
                .any(|p| line.starts_with(p))
        {
            Some(Tok::Meta)
        } else if line.first() == Some(&b'+') || (!in_hunk && line.first() == Some(&b'>')) {
            Some(Tok::Inserted)
        } else if line.first() == Some(&b'-') || (!in_hunk && line.first() == Some(&b'<')) {
            Some(Tok::Deleted)
        } else {
            None
        };
        if let Some(t) = tok {
            o.push(i, end, t);
        }
        i = nx;
    }
}

const DOCKER_KW: &[&str] = &[
    "from",
    "run",
    "cmd",
    "label",
    "maintainer",
    "expose",
    "env",
    "add",
    "copy",
    "entrypoint",
    "volume",
    "user",
    "workdir",
    "arg",
    "onbuild",
    "stopsignal",
    "healthcheck",
    "shell",
];

fn docker(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut line_start = true;
    let mut from_line = false;
    while i < n {
        let c = b[i];
        if c == b'\n' {
            let mut k = i;
            while k > 0 && matches!(b[k - 1], b'\r' | b' ' | b'\t') {
                k -= 1;
            }
            line_start = !(k > 0 && b[k - 1] == b'\\');
            from_line = false;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if line_start {
            line_start = false;
            if c == b'#' {
                let e = eol(b, i);
                o.push(i, trim_cr(b, i, e), Tok::Comment);
                i = e;
                continue;
            }
            if c.is_ascii_alphabetic() {
                let mut j = i;
                while j < n && b[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let w = code.get(i..j).unwrap_or("");
                if has_ci(DOCKER_KW, w) {
                    from_line = w.eq_ignore_ascii_case("from");
                    o.push(i, j, Tok::Keyword);
                    i = j;
                    continue;
                }
            }
        }
        if c == b'"' || c == b'\'' {
            let e = scan_quoted(b, i, c, false);
            o.push(i, e, Tok::String);
            i = e;
        } else if c == b'$' && (at(b, i + 1) == b'{' || is_id_start(at(b, i + 1))) {
            let mut j = i + 1;
            if b[j] == b'{' {
                while j < n && b[j] != b'}' && b[j] != b'\n' {
                    j += 1;
                }
                if at(b, j) == b'}' {
                    j += 1;
                }
            } else {
                while j < n && is_id(b[j]) {
                    j += 1;
                }
            }
            o.push(i, j, Tok::Variable);
            i = j;
        } else if c == b'-' && at(b, i + 1) == b'-' && i > 0 && b[i - 1].is_ascii_whitespace() {
            let mut j = i + 2;
            while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
            o.push(i, j, Tok::Property);
            i = j;
        } else if is_id_start(c) {
            let mut j = i;
            while j < n && is_id(b[j]) {
                j += 1;
            }
            if from_line && code.get(i..j).map_or(false, |w| w.eq_ignore_ascii_case("as")) {
                o.push(i, j, Tok::Keyword);
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

const MAKE_DIRECTIVES: &[&str] = &[
    "ifeq", "ifneq", "ifdef", "ifndef", "else", "endif", "include", "-include", "sinclude", "define", "endef", "export", "unexport",
    "override", "vpath", "undefine",
];
const MAKE_FUNCS: &[&str] = &[
    "call",
    "shell",
    "wildcard",
    "patsubst",
    "subst",
    "filter",
    "filter-out",
    "foreach",
    "if",
    "eval",
    "sort",
    "word",
    "words",
    "strip",
    "dir",
    "notdir",
    "basename",
    "suffix",
    "addprefix",
    "addsuffix",
    "join",
    "abspath",
    "realpath",
    "origin",
    "value",
    "or",
    "and",
    "findstring",
    "firstword",
    "lastword",
    "wordlist",
    "error",
    "warning",
    "info",
];

fn make_inline(b: &[u8], s: usize, e: usize, o: &mut Out) {
    let mut j = s;
    while j < e {
        let c = b[j];
        if c == b'#' && (j == s || b[j - 1] == b' ' || b[j - 1] == b'\t') {
            o.push(j, e, Tok::Comment);
            return;
        }
        if c != b'$' {
            j += 1;
            continue;
        }
        let c1 = at(b, j + 1);
        if c1 == b'(' || c1 == b'{' {
            let close = if c1 == b'(' { b')' } else { b'}' };
            let mut k = j + 2;
            while k < e && (b[k].is_ascii_alphanumeric() || matches!(b[k], b'-' | b'_' | b'.')) {
                k += 1;
            }
            let word = std::str::from_utf8(&b[j + 2..k]).unwrap_or("");
            if at(b, k) == b' ' && MAKE_FUNCS.contains(&word) {
                o.push(j + 2, k, Tok::Function);
                j = k;
                continue;
            }
            let mut depth = 1;
            let mut p = j + 2;
            while p < e {
                if b[p] == close {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                } else if b[p] == c1 {
                    depth += 1;
                }
                p += 1;
            }
            if p < e {
                o.push(j, p + 1, Tok::Variable);
                j = p + 1;
            } else {
                j += 2;
            }
        } else if c1 == b'$' {
            j += 2;
        } else if c1.is_ascii_alphanumeric() || matches!(c1, b'@' | b'<' | b'^' | b'?' | b'*' | b'%' | b'+') {
            o.push(j, j + 2, Tok::Variable);
            j += 2;
        } else {
            j += 1;
        }
    }
}

fn makefile(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        let le = eol(b, i);
        let end = trim_cr(b, i, le);
        let nx = if le < n { le + 1 } else { n };
        let ls = i;
        i = nx;
        if b[ls..end].first() == Some(&b'\t') {
            make_inline(b, ls, end, o);
            continue;
        }
        let mut p = ls;
        while p < end && (b[p] == b' ' || b[p] == b'\t') {
            p += 1;
        }
        if p >= end {
            continue;
        }
        if b[p] == b'#' {
            o.push(p, end, Tok::Comment);
            continue;
        }
        let mut q = p;
        while q < end && !b[q].is_ascii_whitespace() {
            q += 1;
        }
        if std::str::from_utf8(&b[p..q]).map_or(false, |w| MAKE_DIRECTIVES.contains(&w)) {
            o.push(p, q, Tok::Keyword);
            make_inline(b, q, end, o);
            continue;
        }
        // Find the first top-level ':' or '=' (skipping $(...) groups).
        let mut k = p;
        let mut depth = 0usize;
        let mut hit = None;
        while k < end {
            match b[k] {
                b'$' if matches!(at(b, k + 1), b'(' | b'{') => {
                    depth += 1;
                    k += 1;
                }
                b'(' | b'{' if depth > 0 => depth += 1,
                b')' | b'}' if depth > 0 => depth -= 1,
                b'#' if depth == 0 => break,
                b':' | b'=' if depth == 0 => {
                    hit = Some(k);
                    break;
                }
                _ => {}
            }
            k += 1;
        }
        match hit {
            Some(k) if b[k] == b'=' || at(b, k + 1) == b'=' || (at(b, k + 1) == b':' && at(b, k + 2) == b'=') => {
                let mut ns = k;
                if b[k] == b'=' && k > p && matches!(b[k - 1], b'?' | b'+' | b'!' | b':') {
                    ns = k - 1;
                }
                while ns > p && b[ns - 1] == b' ' {
                    ns -= 1;
                }
                o.push(p, ns, Tok::Variable);
                make_inline(b, k + 1, end, o);
            }
            Some(k) => {
                let mut w = p;
                while w < k {
                    while w < k && b[w] == b' ' {
                        w += 1;
                    }
                    let ws = w;
                    while w < k && b[w] != b' ' {
                        w += 1;
                    }
                    if b[ws..w].contains(&b'$') {
                        make_inline(b, ws, w, o);
                    } else {
                        o.push(ws, w, Tok::Function);
                    }
                }
                make_inline(b, k + 1, end, o);
            }
            None => make_inline(b, p, end, o),
        }
    }
}

fn markdown(code: &str, o: &mut Out) {
    let b = code.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut fence = false;
    while i < n {
        let le = eol(b, i);
        let end = trim_cr(b, i, le);
        let nx = if le < n { le + 1 } else { n };
        let ls = i;
        i = nx;
        let mut p = ls;
        while p < end && p < ls + 3 && b[p] == b' ' {
            p += 1;
        }
        if b[p..end].starts_with(b"```") || b[p..end].starts_with(b"~~~") {
            fence = !fence;
            o.push(p, end, Tok::Meta);
            continue;
        }
        if fence {
            continue;
        }
        let mut h = p;
        while h < end && b[h] == b'#' {
            h += 1;
        }
        if h > p && h - p <= 6 && (h >= end || b[h] == b' ') {
            o.push(p, end, Tok::Keyword);
            continue;
        }
        let cap = |from: usize| (from + 1000).min(end);
        let mut j = p;
        while j < end {
            if b[j] == b'`' {
                let mut r = j;
                while r < end && b[r] == b'`' {
                    r += 1;
                }
                let run = r - j;
                let lim = cap(r);
                let mut k = r;
                let mut close = None;
                while k < lim {
                    if b[k] == b'`' {
                        let mut r2 = k;
                        while r2 < end && b[r2] == b'`' {
                            r2 += 1;
                        }
                        if r2 - k == run {
                            close = Some(r2);
                            break;
                        }
                        k = r2;
                    } else {
                        k += 1;
                    }
                }
                if let Some(e) = close {
                    o.push(j, e, Tok::String);
                    j = e;
                } else {
                    j = r;
                }
            } else if b[j] == b'[' {
                let lim = cap(j);
                let mut k = j + 1;
                while k < lim && b[k] != b']' {
                    k += 1;
                }
                if k < lim && at(b, k + 1) == b'(' {
                    let lim2 = cap(k);
                    let mut m = k + 2;
                    while m < lim2 && b[m] != b')' {
                        m += 1;
                    }
                    if m < lim2 {
                        o.push(j, m + 1, Tok::Property);
                        j = m + 1;
                        continue;
                    }
                }
                j += 1;
            } else {
                j += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(lang: &str, code: &str) -> Vec<(String, Tok)> {
        highlight(lang, code).into_iter().map(|(r, t)| (code[r].to_string(), t)).collect()
    }
    fn has_tok(lang: &str, code: &str, s: &str, t: Tok) -> bool {
        toks(lang, code).iter().any(|(x, y)| x == s && *y == t)
    }
    fn check(lang: &str, code: &str, want: &[(&str, Tok)]) {
        for (s, t) in want {
            assert!(has_tok(lang, code, s, *t), "{lang}: expected {s:?} as {t:?} in {:?}", toks(lang, code));
        }
    }
    fn invariants(code: &str, v: &[(Range<usize>, Tok)]) {
        let mut last = 0;
        for (r, _) in v {
            assert!(r.start < r.end && r.start >= last && r.end <= code.len(), "bad range {r:?}");
            assert!(code.is_char_boundary(r.start) && code.is_char_boundary(r.end), "boundary {r:?}");
            last = r.end;
        }
    }

    #[test]
    fn names() {
        assert_eq!(Tok::ALL.len(), 13);
        assert_eq!(Tok::Inserted.name(), "inserted");
        assert_eq!(Tok::Variable.name(), "variable");
    }

    #[test]
    fn rust_family() {
        let src = "#[derive(Debug)]\nfn main<'a>(x: &'a str) {\n    let n = 0xFF_u8; // note\n    println!(\"hi\\n\");\n    let c = 'x'; let r = r#\"raw \"q\" \"#;\n    Foo::new(); /* blk\n more */ MAX_LEN\n}";
        check(
            "rust",
            src,
            &[
                ("#[derive(Debug)]", Tok::Meta),
                ("fn", Tok::Keyword),
                ("main", Tok::Function),
                ("'a", Tok::Meta),
                ("str", Tok::Type),
                ("let", Tok::Keyword),
                ("0xFF_u8", Tok::Number),
                ("// note", Tok::Comment),
                ("println", Tok::Function),
                ("\"hi\\n\"", Tok::String),
                ("'x'", Tok::String),
                ("r#\"raw \"q\" \"#", Tok::String),
                ("Foo", Tok::Type),
                ("new", Tok::Function),
                ("/* blk\n more */", Tok::Comment),
                ("MAX_LEN", Tok::Constant),
            ],
        );
        assert!(highlight("RS", "fn x() {}").len() > 0);
    }

    #[test]
    fn c_family_misc() {
        check(
            "c",
            "#include <stdio.h>\nint main(void) { return NULL; }",
            &[
                ("#include <stdio.h>", Tok::Meta),
                ("int", Tok::Type),
                ("main", Tok::Function),
                ("return", Tok::Keyword),
                ("NULL", Tok::Constant),
            ],
        );
        check(
            "c++",
            "class Foo : public Bar { std::string s; };",
            &[("class", Tok::Keyword), ("Foo", Tok::Type), ("public", Tok::Keyword)],
        );
        check(
            "cs",
            "public async Task Run() { var x = 1.5f; }",
            &[("public", Tok::Keyword), ("Task", Tok::Type), ("Run", Tok::Function), ("1.5f", Tok::Number)],
        );
        check(
            "java",
            "@Override\npublic void run() { String s = \"\"\"\n  text\n\"\"\"; }",
            &[("@Override", Tok::Meta), ("void", Tok::Type), ("\"\"\"\n  text\n\"\"\"", Tok::String)],
        );
        check("kt", "fun main() { val x = null }", &[("fun", Tok::Keyword), ("main", Tok::Function), ("null", Tok::Constant)]);
        check(
            "swift",
            "func f() -> Int { nil }",
            &[("func", Tok::Keyword), ("f", Tok::Function), ("Int", Tok::Type), ("nil", Tok::Constant)],
        );
        check(
            "go",
            "func (s *S) Do() { x := `raw\nline` }",
            &[("func", Tok::Keyword), ("Do", Tok::Function), ("`raw\nline`", Tok::String)],
        );
        check("dart", "void main() { final x = 1; }", &[("void", Tok::Type), ("final", Tok::Keyword)]);
        check("scala", "object App { def f = 1 }", &[("object", Tok::Keyword), ("App", Tok::Type), ("f", Tok::Function)]);
        check("zig", "const x: u8 = @import(\"std\");", &[("const", Tok::Keyword), ("u8", Tok::Type), ("@import", Tok::Meta)]);
        check(
            "php",
            "<?php $a = foo($b); // c\n?>",
            &[("<?php", Tok::Meta), ("$a", Tok::Variable), ("foo", Tok::Function), ("// c", Tok::Comment), ("?>", Tok::Meta)],
        );
    }

    #[test]
    fn js_ts() {
        let src = "const a = `x ${b + `in${c}`} y ${ {k:1}.k }`; // done\nfunction f(n: number) { return null }";
        check(
            "js",
            src,
            &[
                ("const", Tok::Keyword),
                ("`x ${", Tok::String),
                ("} y ${", Tok::String),
                ("}`", Tok::String),
                ("// done", Tok::Comment),
                ("f", Tok::Function),
                ("null", Tok::Constant),
            ],
        );
        // identifiers inside interpolation are not strings; numbers in it are numbers
        assert!(has_tok("js", "`a ${ 42 } b`", "42", Tok::Number));
        check("ts", "interface Foo { a: string }", &[("interface", Tok::Keyword), ("Foo", Tok::Type), ("string", Tok::Type)]);
        check("tsx", "@Component() class A {}", &[("@Component", Tok::Meta)]);
        // unterminated template runs to the end
        let t = toks("js", "`abc ${");
        assert_eq!(t.last().unwrap().1, Tok::String);
    }

    #[test]
    fn python_family() {
        let src = "@dec.x\ndef foo(a):\n    \"\"\"doc\nstring\"\"\"\n    s = f'v{a}' + rb\"x\"  # c\n    return None or True\nclass Bar: pass\nx = 1_000";
        check(
            "py",
            src,
            &[
                ("@dec.x", Tok::Meta),
                ("def", Tok::Keyword),
                ("foo", Tok::Function),
                ("\"\"\"doc\nstring\"\"\"", Tok::String),
                ("f'v{a}'", Tok::String),
                ("rb\"x\"", Tok::String),
                ("# c", Tok::Comment),
                ("None", Tok::Constant),
                ("True", Tok::Constant),
                ("Bar", Tok::Type),
                ("1_000", Tok::Number),
            ],
        );
    }

    #[test]
    fn shell_family() {
        let src = "#!/bin/sh\nexport FOO=\"a b\"\nif [ -f $HOME/x ]; then echo ${BAR} --flag 'q'; fi # end\nfoo() { :; }";
        check(
            "bash",
            src,
            &[
                ("#!/bin/sh", Tok::Comment),
                ("export", Tok::Keyword),
                ("FOO", Tok::Variable),
                ("\"a b\"", Tok::String),
                ("if", Tok::Keyword),
                ("-f", Tok::Property),
                ("$HOME", Tok::Variable),
                ("then", Tok::Keyword),
                ("${BAR}", Tok::Variable),
                ("--flag", Tok::Property),
                ("'q'", Tok::String),
                ("fi", Tok::Keyword),
                ("# end", Tok::Comment),
                ("foo", Tok::Function),
            ],
        );
        let t = toks("console", "$ ls -la\n$ echo $?");
        assert!(!t.iter().any(|(s, _)| s == "$ "));
        assert!(has_tok("console", "$ echo $?", "$?", Tok::Variable));
    }

    #[test]
    fn powershell_family() {
        let src = "# c\n<# block\n#>\nfunction Do-It { param($x) Get-ChildItem -Path $env:Temp -Recurse | Where { $_ -eq $true } }\n$s = \"a`\"b\"; [string]$n = 5";
        check(
            "ps1",
            src,
            &[
                ("# c", Tok::Comment),
                ("<# block\n#>", Tok::Comment),
                ("function", Tok::Keyword),
                ("Do-It", Tok::Function),
                ("param", Tok::Keyword),
                ("$x", Tok::Variable),
                ("Get-ChildItem", Tok::Function),
                ("-Path", Tok::Property),
                ("$env:Temp", Tok::Variable),
                ("-eq", Tok::Keyword),
                ("$true", Tok::Constant),
                ("\"a`\"b\"", Tok::String),
                ("string", Tok::Type),
                ("5", Tok::Number),
            ],
        );
    }

    #[test]
    fn data_formats() {
        check(
            "json",
            "{\"a\": [1, -2.5e3, true, null], \"b\" : \"s\"}",
            &[
                ("\"a\"", Tok::Property),
                ("1", Tok::Number),
                ("-2.5e3", Tok::Number),
                ("true", Tok::Constant),
                ("null", Tok::Constant),
                ("\"b\"", Tok::Property),
                ("\"s\"", Tok::String),
            ],
        );
        check("jsonc", "{ // c\n /* d */ \"k\": 1 }", &[("// c", Tok::Comment), ("/* d */", Tok::Comment)]);
        let y = "---\n# top\nname: \"x\"\nlist:\n  - a: 1\n  - b: true  # t\n  - plain\ntext: |\n  key: not a key\n  more\nnext: ~\nurl: http://x.y/z";
        check(
            "yml",
            y,
            &[
                ("---", Tok::Meta),
                ("# top", Tok::Comment),
                ("name", Tok::Property),
                ("\"x\"", Tok::String),
                ("list", Tok::Property),
                ("a", Tok::Property),
                ("1", Tok::Number),
                ("true", Tok::Constant),
                ("# t", Tok::Comment),
                ("|", Tok::Meta),
                ("key: not a key", Tok::String),
                ("more", Tok::String),
                ("next", Tok::Property),
                ("~", Tok::Constant),
                ("url", Tok::Property),
            ],
        );
        assert!(!has_tok("yaml", "- plain", "plain", Tok::Property));
        let t = "# c\n[pkg.sub]\nname = \"x\" # tail\nn = 1_000\nd = 1979-05-27T07:32:00Z\nok = true\nml = \"\"\"\na\nb\"\"\"\nafter = 2";
        check(
            "toml",
            t,
            &[
                ("# c", Tok::Comment),
                ("[pkg.sub]", Tok::Tag),
                ("name", Tok::Property),
                ("\"x\"", Tok::String),
                ("# tail", Tok::Comment),
                ("1_000", Tok::Number),
                ("1979-05-27T07:32:00Z", Tok::Number),
                ("true", Tok::Constant),
                ("\"\"\"\na\nb\"\"\"", Tok::String),
                ("after", Tok::Property),
            ],
        );
        check(
            "ini",
            "; c\n[s]\nkey=value\nflag = yes ; x\nnum: 12",
            &[
                ("; c", Tok::Comment),
                ("[s]", Tok::Tag),
                ("key", Tok::Property),
                ("yes", Tok::Constant),
                ("; x", Tok::Comment),
                ("num", Tok::Property),
                ("12", Tok::Number),
            ],
        );
    }

    #[test]
    fn markup_family() {
        let h = "<!DOCTYPE html>\n<!-- c -->\n<div class=\"a\" id=b disabled>x</div><br/>\n<script>let a = 1; // s\n</script><style>p { color: #fff; }</style>";
        check(
            "html",
            h,
            &[
                ("<!DOCTYPE html>", Tok::Meta),
                ("<!-- c -->", Tok::Comment),
                ("<div", Tok::Tag),
                ("class", Tok::Property),
                ("\"a\"", Tok::String),
                ("b", Tok::String),
                ("disabled", Tok::Property),
                ("</div", Tok::Tag),
                ("/>", Tok::Tag),
                ("let", Tok::Keyword),
                ("1", Tok::Number),
                ("// s", Tok::Comment),
                ("color", Tok::Property),
                ("#fff", Tok::Constant),
            ],
        );
        let c = "/* c */\n@media (min-width: 10px) { a:hover, .b > #c { margin: -1.5em 10px; color: red !important; width: calc(1px + 2%); background: url(\"x\") } }";
        check(
            "css",
            c,
            &[
                ("/* c */", Tok::Comment),
                ("@media", Tok::Keyword),
                ("a", Tok::Tag),
                (":hover", Tok::Tag),
                (".b", Tok::Tag),
                ("#c", Tok::Tag),
                ("margin", Tok::Property),
                ("-1.5em", Tok::Number),
                ("10px", Tok::Number),
                ("!important", Tok::Keyword),
                ("calc", Tok::Function),
                ("2%", Tok::Number),
                ("\"x\"", Tok::String),
            ],
        );
        check(
            "scss",
            "$c: #abc;\n.a { &:hover { color: $c; } // x\n}",
            &[
                ("$c", Tok::Variable),
                ("#abc", Tok::Constant),
                ("&", Tok::Tag),
                (":hover", Tok::Tag),
                ("color", Tok::Property),
                ("// x", Tok::Comment),
            ],
        );
    }

    #[test]
    fn sql_diff_docker_make_md() {
        check(
            "sql",
            "SELECT count(*), 'it''s' FROM t -- c\nWHERE x = 1 /* b */ AND y IS NULL; -- end",
            &[
                ("SELECT", Tok::Keyword),
                ("count", Tok::Function),
                ("'it''s'", Tok::String),
                ("-- c", Tok::Comment),
                ("1", Tok::Number),
                ("/* b */", Tok::Comment),
                ("NULL", Tok::Constant),
            ],
        );
        let d = "diff --git a/x b/x\nindex 1..2\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n context\n--- sql comment";
        let t = toks("diff", d);
        let get = |s: &str| t.iter().find(|(x, _)| x == s).map(|(_, k)| *k);
        assert_eq!(get("diff --git a/x b/x"), Some(Tok::Meta));
        assert_eq!(get("--- a/x"), Some(Tok::Meta));
        assert_eq!(get("+++ b/x"), Some(Tok::Meta));
        assert_eq!(get("@@ -1,2 +1,2 @@"), Some(Tok::Meta));
        assert_eq!(get("-old"), Some(Tok::Deleted));
        assert_eq!(get("+new"), Some(Tok::Inserted));
        assert_eq!(get(" context"), None);
        assert_eq!(get("--- sql comment"), Some(Tok::Deleted));
        let dk = "# c\nFROM rust:1 AS build\nRUN --mount=x echo \"$A\" ${B} \\\n  && run\nCOPY . .";
        check(
            "dockerfile",
            dk,
            &[
                ("# c", Tok::Comment),
                ("FROM", Tok::Keyword),
                ("AS", Tok::Keyword),
                ("RUN", Tok::Keyword),
                ("--mount", Tok::Property),
                ("\"$A\"", Tok::String),
                ("${B}", Tok::Variable),
                ("COPY", Tok::Keyword),
            ],
        );
        assert!(!has_tok("docker", "RUN a \\\n  COPY b", "COPY", Tok::Keyword));
        let mk = "# c\nCC := gcc\nall: build $(OBJ)\n\t$(CC) -o $@ $< # x\n.PHONY: all\nifeq ($(A),1)\nendif\nX = $(shell ls)";
        check(
            "make",
            mk,
            &[
                ("# c", Tok::Comment),
                ("CC", Tok::Variable),
                ("all", Tok::Function),
                ("$(OBJ)", Tok::Variable),
                ("$(CC)", Tok::Variable),
                ("$@", Tok::Variable),
                ("$<", Tok::Variable),
                ("# x", Tok::Comment),
                (".PHONY", Tok::Function),
                ("ifeq", Tok::Keyword),
                ("X", Tok::Variable),
                ("shell", Tok::Function),
            ],
        );
        let md = "# Title\ntext `code` and [link](http://x) **b**\n```rust\n# not heading\n```\n## Two";
        check(
            "md",
            md,
            &[
                ("# Title", Tok::Keyword),
                ("`code`", Tok::String),
                ("[link](http://x)", Tok::Property),
                ("## Two", Tok::Keyword),
                ("```rust", Tok::Meta),
            ],
        );
        assert!(!has_tok("md", md, "# not heading", Tok::Keyword));
    }

    #[test]
    fn lua_ruby_perl() {
        check(
            "lua",
            "-- c\nlocal s = [[long\nstr]] --[[ blk\n]] function f() return nil end",
            &[
                ("-- c", Tok::Comment),
                ("local", Tok::Keyword),
                ("[[long\nstr]]", Tok::String),
                ("--[[ blk\n]]", Tok::Comment),
                ("f", Tok::Function),
                ("nil", Tok::Constant),
            ],
        );
        check(
            "rb",
            "def foo; @x = :sym; end # c",
            &[("def", Tok::Keyword), ("foo", Tok::Function), ("@x", Tok::Variable), (":sym", Tok::Constant), ("# c", Tok::Comment)],
        );
        check("perl", "my $x = 'a'; # c", &[("my", Tok::Keyword), ("$x", Tok::Variable), ("# c", Tok::Comment)]);
    }

    #[test]
    fn empty_and_unknown() {
        for l in ["regex", "text", "plaintext", "txt", "output", "log", "", "nonsense", "brainfuck"] {
            assert!(highlight(l, "some `text` 123 // x").is_empty(), "{l}");
        }
        assert!(highlight("rust", "").is_empty());
        assert!(!highlight("  Rust ", "fn a() {}").is_empty());
    }

    #[test]
    fn unicode_boundaries() {
        let samples = [
            "let s = \"héllo 😀\"; // ünï 🎉 comment\nlet 名前 = 'é';",
            "# 😀 comment\nx = \"é😀\" + '\\😀' \"\"\"é\n😀\"\"\"",
            "echo \"😀 é\" $é #😀\n\\😀",
            "{\"é😀\": \"😀\", \"k\": 1} // 🎉",
            "<p title=\"é😀\">😀</p><!-- 🎉 -->",
            "a { content: \"😀\"; /* é */ } .é { x: 1 }",
            "key: \"😀\" # é\n😀: é",
            "SELECT 'é😀' -- 🎉\n, \"é\"",
            "+é😀\n-😀\n@@ é @@",
            "# é\n`😀` [é](😀)",
        ];
        let langs = ["rust", "py", "sh", "json", "html", "css", "yaml", "sql", "diff", "md", "js", "ps1", "toml", "make", "docker", "lua"];
        for s in samples {
            for l in langs {
                invariants(s, &highlight(l, s));
            }
        }
    }

    const ALIASES: &[&str] = &[
        "rust",
        "rs",
        "c",
        "h",
        "cpp",
        "c++",
        "cc",
        "hpp",
        "cxx",
        "cs",
        "csharp",
        "c#",
        "java",
        "kotlin",
        "kt",
        "swift",
        "go",
        "golang",
        "javascript",
        "js",
        "jsx",
        "mjs",
        "cjs",
        "typescript",
        "ts",
        "tsx",
        "dart",
        "scala",
        "php",
        "zig",
        "python",
        "py",
        "bash",
        "sh",
        "shell",
        "zsh",
        "console",
        "shell-session",
        "powershell",
        "ps1",
        "pwsh",
        "ps",
        "json",
        "jsonc",
        "json5",
        "yaml",
        "yml",
        "toml",
        "ini",
        "cfg",
        "conf",
        "html",
        "xml",
        "svg",
        "xhtml",
        "vue",
        "css",
        "scss",
        "less",
        "sql",
        "diff",
        "patch",
        "dockerfile",
        "docker",
        "makefile",
        "make",
        "lua",
        "ruby",
        "rb",
        "perl",
        "markdown",
        "md",
        "regex",
        "text",
        "plaintext",
        "txt",
        "output",
        "log",
        "",
    ];

    #[test]
    fn no_panic_every_prefix() {
        let sample = "#!/bin/sh\r\n// c /* open\r\n/* unterminated \"str 'q' \\\\ \\\r\n\"é😀\" 'x' `tpl ${a} \\` r#\"raw\r\n\"\"\"tri\r\n<!-- x <a b=\"c\r\n<script>let a=`${\r\n@@ -1 +1 @@\r\n- 😀 $(x) ${y} <# z\r\n[sec]\r\nk: |\r\n  v\r\n--- 'a''b' [x](y `z\r\n@x :s $v #{ { } }\r\n\tfoo: bar\\\r\n$$ \\";
        for l in ALIASES {
            for (i, _) in sample.char_indices().chain(std::iter::once((sample.len(), ' '))) {
                let p = &sample[..i];
                invariants(p, &highlight(l, p));
            }
            invariants(sample, &highlight(&l.to_ascii_uppercase(), sample));
        }
    }

    #[test]
    fn linear_on_pathological_input() {
        let big = "[`(\"'/*<#$@{".repeat(20_000);
        let start = std::time::Instant::now();
        for l in ALIASES {
            invariants(&big, &highlight(l, &big));
        }
        assert!(start.elapsed().as_secs() < 20, "too slow: {:?}", start.elapsed());
    }
}
