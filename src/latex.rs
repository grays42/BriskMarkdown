//! LaTeX math to readable Unicode text (no layout engine).
//!
//! Single-pass recursive-descent converter. Linear time: every step consumes at
//! least one input character, recursion depth is capped, and unmatched openers
//! consume to the end of input instead of rescanning.

use std::panic::{catch_unwind, AssertUnwindSafe};

/// Convert a LaTeX math fragment to readable Unicode. `display` is true for $$ blocks.
/// Never panics on any input. Unknown commands stay as their literal text (e.g. "\foo").
/// Display math may return several lines separated by '\n' (from \\ in aligned/cases/matrix envs).
pub fn latex_to_unicode(src: &str, display: bool) -> String {
    let r = catch_unwind(AssertUnwindSafe(|| Parser::new(src, display, 0).run()));
    match r {
        Ok(s) => s,
        Err(_) => src.to_string(),
    }
}

const MAX_DEPTH: u32 = 40;
const MAX_NEST: u32 = 6;

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Italic,
    Upright,
    Bold,
    Bb,
    Cal,
    Frak,
}

struct Parser {
    c: Vec<char>,
    p: usize,
    display: bool,
    depth: u32,
    nest: u32,
    style: Style,
    in_env: u32,
}

// ---------------------------------------------------------------- tables

const GREEK: &[(&str, &str)] = &[
    ("alpha", "α"),
    ("beta", "β"),
    ("gamma", "γ"),
    ("delta", "δ"),
    ("epsilon", "ϵ"),
    ("varepsilon", "ε"),
    ("zeta", "ζ"),
    ("eta", "η"),
    ("theta", "θ"),
    ("vartheta", "ϑ"),
    ("iota", "ι"),
    ("kappa", "κ"),
    ("varkappa", "ϰ"),
    ("lambda", "λ"),
    ("mu", "μ"),
    ("nu", "ν"),
    ("xi", "ξ"),
    ("omicron", "ο"),
    ("pi", "π"),
    ("varpi", "ϖ"),
    ("rho", "ρ"),
    ("varrho", "ϱ"),
    ("sigma", "σ"),
    ("varsigma", "ς"),
    ("tau", "τ"),
    ("upsilon", "υ"),
    ("phi", "ϕ"),
    ("varphi", "φ"),
    ("chi", "χ"),
    ("psi", "ψ"),
    ("omega", "ω"),
    ("digamma", "ϝ"),
    ("Gamma", "Γ"),
    ("Delta", "Δ"),
    ("Theta", "Θ"),
    ("Lambda", "Λ"),
    ("Xi", "Ξ"),
    ("Pi", "Π"),
    ("Sigma", "Σ"),
    ("Upsilon", "Υ"),
    ("Phi", "Φ"),
    ("Psi", "Ψ"),
    ("Omega", "Ω"),
];

const ORDS: &[(&str, &str)] = &[
    ("infty", "∞"),
    ("partial", "∂"),
    ("nabla", "∇"),
    ("forall", "∀"),
    ("exists", "∃"),
    ("nexists", "∄"),
    ("neg", "¬"),
    ("lnot", "¬"),
    ("emptyset", "∅"),
    ("varnothing", "∅"),
    ("angle", "∠"),
    ("measuredangle", "∡"),
    ("sphericalangle", "∢"),
    ("triangle", "△"),
    ("hbar", "ℏ"),
    ("ell", "ℓ"),
    ("Re", "ℜ"),
    ("Im", "ℑ"),
    ("aleph", "ℵ"),
    ("beth", "ℶ"),
    ("wp", "℘"),
    ("top", "⊤"),
    ("bot", "⊥"),
    ("prime", "′"),
    ("dprime", "″"),
    ("ldots", "…"),
    ("dots", "…"),
    ("dotsc", "…"),
    ("cdots", "⋯"),
    ("dotsb", "⋯"),
    ("dotsm", "⋯"),
    ("vdots", "⋮"),
    ("ddots", "⋱"),
    ("imath", "ı"),
    ("jmath", "ȷ"),
    ("surd", "√"),
    ("checkmark", "✓"),
    ("degree", "°"),
    ("textdegree", "°"),
    ("therefore", "∴"),
    ("because", "∵"),
    ("square", "□"),
    ("Box", "□"),
    ("blacksquare", "■"),
    ("diamond", "⋄"),
    ("lbrace", "{"),
    ("rbrace", "}"),
    ("lbrack", "["),
    ("rbrack", "]"),
    ("vert", "|"),
    ("Vert", "‖"),
    ("lvert", "|"),
    ("rvert", "|"),
    ("lVert", "‖"),
    ("rVert", "‖"),
    ("langle", "⟨"),
    ("rangle", "⟩"),
    ("lfloor", "⌊"),
    ("rfloor", "⌋"),
    ("lceil", "⌈"),
    ("rceil", "⌉"),
    ("backslash", "\\"),
    ("textbackslash", "\\"),
    ("uparrow", "↑"),
    ("downarrow", "↓"),
    ("updownarrow", "↕"),
    ("Uparrow", "⇑"),
    ("Downarrow", "⇓"),
    ("nearrow", "↗"),
    ("searrow", "↘"),
    ("swarrow", "↙"),
    ("nwarrow", "↖"),
    ("S", "§"),
    ("P", "¶"),
    ("dag", "†"),
    ("ddag", "‡"),
    ("dagger", "†"),
    ("ddagger", "‡"),
    ("copyright", "©"),
    ("pounds", "£"),
    ("mho", "℧"),
    ("complement", "∁"),
    ("clubsuit", "♣"),
    ("diamondsuit", "♢"),
    ("heartsuit", "♡"),
    ("spadesuit", "♠"),
    ("flat", "♭"),
    ("natural", "♮"),
    ("sharp", "♯"),
    ("colon", ":"),
    ("lt", "<"),
    ("gt", ">"),
];

const BINS: &[(&str, &str)] = &[
    ("cdot", "·"),
    ("times", "×"),
    ("div", "÷"),
    ("pm", "±"),
    ("mp", "∓"),
    ("cup", "∪"),
    ("cap", "∩"),
    ("land", "∧"),
    ("lor", "∨"),
    ("wedge", "∧"),
    ("vee", "∨"),
    ("oplus", "⊕"),
    ("otimes", "⊗"),
    ("ominus", "⊖"),
    ("oslash", "⊘"),
    ("odot", "⊙"),
    ("circ", "∘"),
    ("star", "⋆"),
    ("ast", "∗"),
    ("setminus", "∖"),
    ("smallsetminus", "∖"),
    ("bullet", "∙"),
    ("uplus", "⊎"),
    ("sqcup", "⊔"),
    ("sqcap", "⊓"),
    ("amalg", "⨿"),
    ("bigcirc", "○"),
    ("triangleleft", "◁"),
    ("triangleright", "▷"),
    ("centerdot", "·"),
    ("ltimes", "⋉"),
    ("rtimes", "⋊"),
    ("wr", "≀"),
    ("intercal", "⊺"),
    ("lhd", "⊲"),
    ("rhd", "⊳"),
];

const RELS: &[(&str, &str)] = &[
    ("le", "≤"),
    ("leq", "≤"),
    ("leqslant", "≤"),
    ("ge", "≥"),
    ("geq", "≥"),
    ("geqslant", "≥"),
    ("neq", "≠"),
    ("ne", "≠"),
    ("approx", "≈"),
    ("approxeq", "≊"),
    ("equiv", "≡"),
    ("sim", "∼"),
    ("simeq", "≃"),
    ("cong", "≅"),
    ("propto", "∝"),
    ("ll", "≪"),
    ("gg", "≫"),
    ("lll", "⋘"),
    ("ggg", "⋙"),
    ("in", "∈"),
    ("notin", "∉"),
    ("ni", "∋"),
    ("owns", "∋"),
    ("subset", "⊂"),
    ("subseteq", "⊆"),
    ("supset", "⊃"),
    ("supseteq", "⊇"),
    ("subsetneq", "⊊"),
    ("supsetneq", "⊋"),
    ("nsubseteq", "⊈"),
    ("nsupseteq", "⊉"),
    ("nsubset", "⊄"),
    ("nsupset", "⊅"),
    ("sqsubset", "⊏"),
    ("sqsupset", "⊐"),
    ("sqsubseteq", "⊑"),
    ("sqsupseteq", "⊒"),
    ("to", "→"),
    ("rightarrow", "→"),
    ("leftarrow", "←"),
    ("Rightarrow", "⇒"),
    ("Leftarrow", "⇐"),
    ("iff", "⟺"),
    ("Leftrightarrow", "⇔"),
    ("leftrightarrow", "↔"),
    ("mapsto", "↦"),
    ("longmapsto", "⟼"),
    ("implies", "⟹"),
    ("impliedby", "⟸"),
    ("longrightarrow", "⟶"),
    ("longleftarrow", "⟵"),
    ("longleftrightarrow", "⟷"),
    ("Longrightarrow", "⟹"),
    ("Longleftarrow", "⟸"),
    ("Longleftrightarrow", "⟺"),
    ("gets", "←"),
    ("hookrightarrow", "↪"),
    ("hookleftarrow", "↩"),
    ("leadsto", "⇝"),
    ("rightleftharpoons", "⇌"),
    ("rightharpoonup", "⇀"),
    ("leftharpoonup", "↼"),
    ("perp", "⊥"),
    ("parallel", "∥"),
    ("nparallel", "∦"),
    ("mid", "∣"),
    ("nmid", "∤"),
    ("models", "⊨"),
    ("vdash", "⊢"),
    ("dashv", "⊣"),
    ("vDash", "⊨"),
    ("prec", "≺"),
    ("succ", "≻"),
    ("preceq", "≼"),
    ("succeq", "≽"),
    ("asymp", "≍"),
    ("doteq", "≐"),
    ("nsim", "≁"),
    ("nleq", "≰"),
    ("ngeq", "≱"),
    ("lesssim", "≲"),
    ("gtrsim", "≳"),
    ("lessgtr", "≶"),
    ("gtrless", "≷"),
    ("triangleq", "≜"),
    ("coloneqq", "≔"),
    ("eqqcolon", "≕"),
    ("bowtie", "⋈"),
];

const OPS: &[(&str, &str)] = &[
    ("sum", "∑"),
    ("prod", "∏"),
    ("coprod", "∐"),
    ("int", "∫"),
    ("iint", "∬"),
    ("iiint", "∭"),
    ("oint", "∮"),
    ("oiint", "∯"),
    ("bigcup", "⋃"),
    ("bigcap", "⋂"),
    ("bigoplus", "⨁"),
    ("bigotimes", "⨂"),
    ("bigodot", "⨀"),
    ("biguplus", "⨄"),
    ("bigvee", "⋁"),
    ("bigwedge", "⋀"),
    ("bigsqcup", "⨆"),
];

const FUNCS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth", "log", "ln", "lg", "exp",
    "lim", "limsup", "liminf", "max", "min", "sup", "inf", "det", "gcd", "lcm", "arg", "deg", "dim", "ker", "Pr", "hom", "injlim",
    "projlim",
];

const ACCENTS: &[(&str, char)] = &[
    ("hat", '\u{302}'),
    ("widehat", '\u{302}'),
    ("check", '\u{30C}'),
    ("tilde", '\u{303}'),
    ("widetilde", '\u{303}'),
    ("acute", '\u{301}'),
    ("grave", '\u{300}'),
    ("dot", '\u{307}'),
    ("ddot", '\u{308}'),
    ("dddot", '\u{20DB}'),
    ("breve", '\u{306}'),
    ("bar", '\u{304}'),
    ("overline", '\u{305}'),
    ("underline", '\u{332}'),
    ("vec", '\u{20D7}'),
    ("overrightarrow", '\u{20D7}'),
    ("overleftarrow", '\u{20D6}'),
    ("mathring", '\u{30A}'),
];

const DELIMS: &[&str] =
    &["left", "right", "middle", "big", "Big", "bigg", "Bigg", "bigl", "bigr", "Bigl", "Bigr", "biggl", "biggr", "Biggl", "Biggr"];

/// Commands that produce nothing and take no argument.
const DROP: &[&str] = &[
    "displaystyle",
    "textstyle",
    "scriptstyle",
    "scriptscriptstyle",
    "limits",
    "nolimits",
    "nonumber",
    "notag",
    "hline",
    "hdashline",
    "mathstrut",
    "relax",
    "bigskip",
    "medskip",
    "smallskip",
    "noindent",
    "centering",
    "allowbreak",
    "protect",
    "strut",
    "nobreak",
];

/// Commands that produce nothing and swallow one braced argument.
const DROP1: &[&str] = &["label", "vspace", "phantom", "hphantom", "vphantom", "ref", "eqref", "cite", "pagebreak"];

/// Commands that take one math argument and pass it through unchanged.
const PASS: &[&str] = &[
    "mathrel",
    "mathbin",
    "mathord",
    "mathopen",
    "mathclose",
    "mathpunct",
    "ensuremath",
    "cancel",
    "bcancel",
    "xcancel",
    "overbrace",
    "underbrace",
    "smash",
];

fn lookup(tbl: &[(&str, &'static str)], n: &str) -> Option<&'static str> {
    tbl.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
}

fn font_of(n: &str) -> Option<Style> {
    Some(match n {
        "mathbb" | "Bbb" | "mathds" | "bbold" => Style::Bb,
        "mathbf" | "boldsymbol" | "bm" | "mathbfit" | "pmb" => Style::Bold,
        "mathcal" | "mathscr" => Style::Cal,
        "mathfrak" => Style::Frak,
        "mathrm" | "mathsf" | "mathtt" | "mathup" => Style::Upright,
        "mathit" | "mathnormal" => Style::Italic,
        _ => return None,
    })
}

// ---------------------------------------------------------------- letters

fn styled_letter(c: char, st: Style) -> char {
    let up = c.is_ascii_uppercase();
    let lo = c.is_ascii_lowercase();
    if !up && !lo {
        return c;
    }
    let (ub, lb): (u32, u32) = match st {
        Style::Upright => return c,
        Style::Italic => {
            if c == 'h' {
                return '\u{210E}';
            }
            (0x1D434, 0x1D44E)
        }
        Style::Bold => (0x1D400, 0x1D41A),
        Style::Bb => {
            let ex = match c {
                'C' => Some('ℂ'),
                'H' => Some('ℍ'),
                'N' => Some('ℕ'),
                'P' => Some('ℙ'),
                'Q' => Some('ℚ'),
                'R' => Some('ℝ'),
                'Z' => Some('ℤ'),
                _ => None,
            };
            if let Some(e) = ex {
                return e;
            }
            (0x1D538, 0x1D552)
        }
        Style::Cal => {
            let ex = match c {
                'B' => Some('ℬ'),
                'E' => Some('ℰ'),
                'F' => Some('ℱ'),
                'H' => Some('ℋ'),
                'I' => Some('ℐ'),
                'L' => Some('ℒ'),
                'M' => Some('ℳ'),
                'R' => Some('ℛ'),
                'e' => Some('ℯ'),
                'g' => Some('ℊ'),
                'o' => Some('ℴ'),
                _ => None,
            };
            if let Some(e) = ex {
                return e;
            }
            (0x1D49C, 0x1D4B6)
        }
        Style::Frak => {
            let ex = match c {
                'C' => Some('ℭ'),
                'H' => Some('ℌ'),
                'I' => Some('ℑ'),
                'R' => Some('ℜ'),
                'Z' => Some('ℨ'),
                _ => None,
            };
            if let Some(e) = ex {
                return e;
            }
            (0x1D504, 0x1D51E)
        }
    };
    let cp = if up { ub + (c as u32 - 'A' as u32) } else { lb + (c as u32 - 'a' as u32) };
    char::from_u32(cp).unwrap_or(c)
}

fn styled_digit(c: char, st: Style) -> char {
    let d = c as u32 - '0' as u32;
    let base = match st {
        Style::Bold => 0x1D7CE,
        Style::Bb => 0x1D7D8,
        _ => return c,
    };
    char::from_u32(base + d).unwrap_or(c)
}

/// Map a styled math letter/digit back to ASCII (for script lookup).
fn unstyle(c: char) -> char {
    let u = c as u32;
    match u {
        0x210E => 'h',
        0x1D434..=0x1D44D => (b'A' + (u - 0x1D434) as u8) as char,
        0x1D44E..=0x1D467 => (b'a' + (u - 0x1D44E) as u8) as char,
        0x1D400..=0x1D419 => (b'A' + (u - 0x1D400) as u8) as char,
        0x1D41A..=0x1D433 => (b'a' + (u - 0x1D41A) as u8) as char,
        0x1D7CE..=0x1D7D7 => (b'0' + (u - 0x1D7CE) as u8) as char,
        0x1D7D8..=0x1D7E1 => (b'0' + (u - 0x1D7D8) as u8) as char,
        _ => c,
    }
}

fn sup_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        'A' => 'ᴬ',
        'B' => 'ᴮ',
        'D' => 'ᴰ',
        'E' => 'ᴱ',
        'G' => 'ᴳ',
        'H' => 'ᴴ',
        'I' => 'ᴵ',
        'J' => 'ᴶ',
        'K' => 'ᴷ',
        'L' => 'ᴸ',
        'M' => 'ᴹ',
        'N' => 'ᴺ',
        'O' => 'ᴼ',
        'P' => 'ᴾ',
        'R' => 'ᴿ',
        'T' => 'ᵀ',
        'U' => 'ᵁ',
        'V' => 'ⱽ',
        'W' => 'ᵂ',
        'β' => 'ᵝ',
        'γ' => 'ᵞ',
        'δ' => 'ᵟ',
        'θ' => 'ᶿ',
        'φ' => 'ᵠ',
        'χ' => 'ᵡ',
        '′' | '″' | '‴' | '°' | '†' | '‡' => c,
        _ => return None,
    })
}

fn sub_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        'β' => 'ᵦ',
        'γ' => 'ᵧ',
        'ρ' => 'ᵨ',
        'φ' => 'ᵩ',
        'χ' => 'ᵪ',
        _ => return None,
    })
}

fn to_script(s: &str, sup: bool) -> Option<String> {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let b = unstyle(ch);
        match if sup { sup_char(b) } else { sub_char(b) } {
            Some(x) => o.push(x),
            None => return None,
        }
    }
    Some(o)
}

fn script(a: &str, sup: bool) -> String {
    let t = a.trim();
    if t.is_empty() {
        return String::new();
    }
    if sup && t == "∘" {
        return "°".to_string();
    }
    if let Some(m) = to_script(t, sup) {
        return m;
    }
    let mark = if sup { '^' } else { '_' };
    if t.chars().count() == 1 {
        format!("{}{}", mark, t)
    } else {
        format!("{}({})", mark, t)
    }
}

fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x300..=0x36F | 0x20D0..=0x20FF)
}

fn balanced_whole(s: &str) -> bool {
    let n = s.chars().count();
    let mut d: i32 = 0;
    for (i, ch) in s.chars().enumerate() {
        if ch == '(' {
            d += 1;
        } else if ch == ')' {
            d -= 1;
            if d < 0 {
                return false;
            }
            if d == 0 && i + 1 < n {
                return false;
            }
        }
    }
    d == 0
}

fn is_atom(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s.starts_with('(') && s.ends_with(')') && balanced_whole(s) {
        return true;
    }
    let t = s.strip_prefix('−').unwrap_or(s);
    !t.is_empty()
        && t.chars().all(|c| {
            c.is_alphanumeric() || c == '.' || is_combining(c) || ('\u{2070}'..='\u{209F}').contains(&c) || c == '′' || c == '″' || c == '°'
        })
}

fn wrap(s: &str) -> String {
    if is_atom(s) {
        s.to_string()
    } else {
        format!("({})", s)
    }
}

fn accent(a: &str, comb: char) -> String {
    let mut o = String::with_capacity(a.len() * 2);
    for ch in a.chars() {
        o.push(ch);
        if !ch.is_whitespace() && !is_combining(ch) {
            o.push(comb);
        }
    }
    o
}

// ---------------------------------------------------------------- output helpers

/// Add a space unless the output is empty or already ends in whitespace.
fn sp(out: &mut String) {
    if let Some(l) = out.chars().next_back() {
        if !l.is_whitespace() {
            out.push(' ');
        }
    }
}

/// Pending space kinds: 1 after a relation/binary operator, 2 after a function
/// name (cancelled before an opening bracket), 3 after an operator or comma.
fn flush(out: &mut String, pend: &mut u8, next: char) {
    let k = *pend;
    *pend = 0;
    if k == 0 || next.is_whitespace() {
        return;
    }
    if k >= 2 {
        if matches!(next, ')' | ']' | ',' | '.' | ';' | '!' | '^' | '_' | '/') {
            return;
        }
        if k == 2 && matches!(next, '(' | '[' | '|') {
            return;
        }
    }
    sp(out);
}

fn emit(out: &mut String, pend: &mut u8, s: &str) {
    if let Some(f) = s.chars().next() {
        flush(out, pend, f);
        out.push_str(s);
    }
}

fn emit_ch(out: &mut String, pend: &mut u8, c: char) {
    let mut b = [0u8; 4];
    emit(out, pend, c.encode_utf8(&mut b));
}

fn rel(out: &mut String, pend: &mut u8, s: &str) {
    if let Some(f) = s.chars().next() {
        flush(out, pend, f);
        sp(out);
        out.push_str(s);
        *pend = 1;
    }
}

fn is_unary(out: &str) -> bool {
    match out.chars().next_back() {
        None => true,
        Some(l) => l.is_whitespace() || "([{,/|=<>".contains(l),
    }
}

fn bin(out: &mut String, pend: &mut u8, s: &str) {
    if let Some(f) = s.chars().next() {
        flush(out, pend, f);
        if is_unary(out) {
            out.push_str(s);
        } else {
            sp(out);
            out.push_str(s);
            *pend = 1;
        }
    }
}

// ---------------------------------------------------------------- parser

impl Parser {
    fn new(src: &str, display: bool, nest: u32) -> Parser {
        Parser { c: src.chars().collect(), p: 0, display, depth: 0, nest, style: Style::Italic, in_env: 0 }
    }

    fn run(&mut self) -> String {
        let s = self.seq(false, false);
        s.trim_matches(|c| c == ' ' || c == '\n').to_string()
    }

    fn peek(&self) -> Option<char> {
        self.c.get(self.p).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.c.get(self.p + n).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.p += 1;
        }
    }

    fn at_env_stop(&self) -> bool {
        match self.peek() {
            Some('&') => true,
            Some('\\') => match self.peek_at(1) {
                Some('\\') => true,
                Some('e') => {
                    self.c[self.p + 1..].starts_with(&['e', 'n', 'd']) && !self.peek_at(4).map_or(false, |c| c.is_ascii_alphabetic())
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn seq(&mut self, brace: bool, env: bool) -> String {
        let mut out = String::new();
        let mut pend = 0u8;
        while let Some(c) = self.peek() {
            if brace && c == '}' {
                break;
            }
            if env && self.at_env_stop() {
                break;
            }
            self.step(&mut out, &mut pend);
        }
        out
    }

    /// One argument: a braced group or a single token. Empty at '}' / end / env separators.
    fn arg(&mut self) -> String {
        self.skip_ws();
        match self.peek() {
            None | Some('}') => String::new(),
            _ if self.in_env > 0 && self.at_env_stop() => String::new(),
            Some('{') => {
                self.p += 1;
                let st = self.style;
                let g = self.seq(true, false);
                self.style = st;
                if self.peek() == Some('}') {
                    self.p += 1;
                }
                g
            }
            Some(_) => {
                let mut o = String::new();
                let mut pd = 0u8;
                self.step(&mut o, &mut pd);
                o
            }
        }
    }

    fn with_style(&mut self, st: Style) -> String {
        let old = self.style;
        self.style = st;
        let a = self.arg();
        self.style = old;
        a
    }

    fn raw_group(&mut self) -> Option<String> {
        self.skip_ws();
        if self.peek() != Some('{') {
            return None;
        }
        self.p += 1;
        let mut depth = 1u32;
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            self.p += 1;
            if ch == '\\' {
                s.push(ch);
                if let Some(n) = self.peek() {
                    s.push(n);
                    self.p += 1;
                }
                continue;
            }
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    return Some(s);
                }
            }
            s.push(ch);
        }
        Some(s)
    }

    /// Skip a short `[...]` directly at the cursor (e.g. `\\[2pt]`).
    fn skip_bracket_opt(&mut self) {
        if self.peek() == Some('[') {
            for k in 1..=16 {
                match self.peek_at(k) {
                    Some(']') => {
                        self.p += k + 1;
                        return;
                    }
                    Some('\\') | None => return,
                    _ => {}
                }
            }
        }
    }

    fn sub_convert(&self, src: &str) -> String {
        if self.nest >= MAX_NEST {
            return src.to_string();
        }
        Parser::new(src, false, self.nest + 1).run()
    }

    fn text_of(&self, raw: &str) -> String {
        let cs: Vec<char> = raw.chars().collect();
        let mut o = String::with_capacity(raw.len());
        let mut i = 0;
        while i < cs.len() {
            let ch = cs[i];
            i += 1;
            match ch {
                '\\' => {
                    let Some(&nc) = cs.get(i) else {
                        o.push('\\');
                        break;
                    };
                    if nc.is_ascii_alphabetic() {
                        let st = i;
                        while i < cs.len() && cs[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        let name: String = cs[st..i].iter().collect();
                        match name.as_str() {
                            "quad" => o.push('\u{2003}'),
                            "qquad" => o.push_str("\u{2003}\u{2003}"),
                            _ => o.push_str(&self.sub_convert(&format!("\\{}", name))),
                        }
                    } else {
                        i += 1;
                        match nc {
                            ',' => o.push('\u{2009}'),
                            '\\' => o.push(' '),
                            _ => o.push(nc),
                        }
                    }
                }
                '$' => {
                    let st = i;
                    while i < cs.len() && cs[i] != '$' {
                        i += 1;
                    }
                    let seg: String = cs[st..i].iter().collect();
                    if i < cs.len() {
                        i += 1;
                    }
                    o.push_str(&self.sub_convert(&seg));
                }
                '~' => o.push('\u{a0}'),
                '{' | '}' => {}
                _ => o.push(ch),
            }
        }
        o
    }

    fn step(&mut self, out: &mut String, pend: &mut u8) {
        let c = self.c[self.p];
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            self.p += 1;
            return;
        }
        if self.depth >= MAX_DEPTH {
            self.p += 1;
            out.push(c);
            return;
        }
        self.depth += 1;
        self.inner(c, out, pend);
        self.depth -= 1;
    }

    fn inner(&mut self, c: char, out: &mut String, pend: &mut u8) {
        match c {
            '\\' => self.command(out, pend),
            '{' => {
                flush(out, pend, '{');
                self.p += 1;
                let st = self.style;
                let g = self.seq(true, false);
                self.style = st;
                if self.peek() == Some('}') {
                    self.p += 1;
                }
                out.push_str(&g);
            }
            '^' | '_' => {
                self.p += 1;
                let a = self.arg();
                out.push_str(&script(&a, c == '^'));
            }
            '\'' => {
                let mut n = 0;
                while self.peek() == Some('\'') {
                    n += 1;
                    self.p += 1;
                }
                match n {
                    1 => out.push('′'),
                    2 => out.push('″'),
                    3 => out.push('‴'),
                    _ => (0..n).for_each(|_| out.push('′')),
                }
            }
            '+' => {
                self.p += 1;
                bin(out, pend, "+");
            }
            '-' => {
                if self.peek_at(1) == Some('>') {
                    self.p += 2;
                    rel(out, pend, "→");
                } else {
                    self.p += 1;
                    bin(out, pend, "−");
                }
            }
            '=' => {
                if self.peek_at(1) == Some('>') {
                    self.p += 2;
                    rel(out, pend, "⇒");
                } else {
                    self.p += 1;
                    rel(out, pend, "=");
                }
            }
            '<' => {
                if self.peek_at(1) == Some('=') {
                    self.p += 2;
                    rel(out, pend, "≤");
                } else {
                    self.p += 1;
                    rel(out, pend, "<");
                }
            }
            '>' => {
                if self.peek_at(1) == Some('=') {
                    self.p += 2;
                    rel(out, pend, "≥");
                } else {
                    self.p += 1;
                    rel(out, pend, ">");
                }
            }
            ':' if self.peek_at(1) == Some('=') => {
                self.p += 2;
                rel(out, pend, "≔");
            }
            '&' => {
                self.p += 1;
                flush(out, pend, ' ');
                out.push(' ');
            }
            '~' => {
                self.p += 1;
                emit_ch(out, pend, '\u{a0}');
            }
            ',' => {
                self.p += 1;
                emit_ch(out, pend, ',');
                *pend = 3;
            }
            _ => {
                self.p += 1;
                if c.is_ascii_alphabetic() {
                    emit_ch(out, pend, styled_letter(c, self.style));
                } else if c.is_ascii_digit() {
                    emit_ch(out, pend, styled_digit(c, self.style));
                } else {
                    emit_ch(out, pend, c);
                    if c == ':' {
                        *pend = 3;
                    }
                }
            }
        }
    }

    fn command(&mut self, out: &mut String, pend: &mut u8) {
        self.p += 1; // the backslash
        let Some(c) = self.peek() else {
            out.push('\\');
            return;
        };
        if !c.is_ascii_alphabetic() {
            self.p += 1;
            match c {
                '\\' => {
                    if self.display {
                        out.push('\n');
                    } else {
                        out.push_str("; ");
                    }
                    *pend = 0;
                    self.skip_bracket_opt();
                }
                ',' => out.push('\u{2009}'),
                ';' => out.push('\u{2004}'),
                ':' | '>' => out.push('\u{2005}'),
                '!' | '/' | '-' | '@' | '(' | ')' | '[' | ']' => {}
                ' ' => out.push(' '),
                '{' | '}' | '%' | '$' | '&' | '#' | '_' => emit_ch(out, pend, c),
                '|' => emit(out, pend, "‖"),
                _ => {
                    out.push('\\');
                    out.push(c);
                }
            }
            return;
        }
        let start = self.p;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphabetic()) {
            self.p += 1;
        }
        let name: String = self.c[start..self.p].iter().collect();
        let n = name.as_str();
        if self.peek() == Some('*') && n == "operatorname" {
            self.p += 1;
        }
        match n {
            "frac" | "dfrac" | "tfrac" | "cfrac" | "nicefrac" => {
                let a = self.arg();
                let b = self.arg();
                let s = format!("{}/{}", wrap(&a), wrap(&b));
                emit(out, pend, &s);
            }
            "binom" | "dbinom" | "tbinom" => {
                let a = self.arg();
                let b = self.arg();
                let s = format!("C({},{})", a, b);
                emit(out, pend, &s);
            }
            "sqrt" => {
                self.skip_ws();
                let mut idx: Option<String> = None;
                if self.peek() == Some('[') {
                    for k in 1..=16 {
                        match self.peek_at(k) {
                            Some(']') => {
                                let raw: String = self.c[self.p + 1..self.p + k].iter().collect();
                                idx = Some(self.sub_convert(&raw));
                                self.p += k + 1;
                                break;
                            }
                            None => break,
                            _ => {}
                        }
                    }
                }
                let a = self.arg();
                let mut s = match idx.as_deref() {
                    None | Some("2") => "√".to_string(),
                    Some("3") => "∛".to_string(),
                    Some("4") => "∜".to_string(),
                    Some(i) => match to_script(i, true) {
                        Some(m) => format!("{}√", m),
                        None => format!("({})√", i),
                    },
                };
                s.push_str(&wrap(&a));
                emit(out, pend, &s);
            }
            "text" | "textrm" | "textbf" | "textit" | "textsf" | "texttt" | "mbox" | "hbox" | "textnormal" | "textup" | "textmd"
            | "emph" => {
                if let Some(raw) = self.raw_group() {
                    let s = self.text_of(&raw);
                    emit(out, pend, &s);
                }
            }
            "operatorname" => {
                if let Some(raw) = self.raw_group() {
                    let s = self.text_of(&raw);
                    emit(out, pend, &s);
                    *pend = 2;
                }
            }
            "mathop" => {
                let a = self.with_style(Style::Upright);
                emit(out, pend, &a);
                *pend = 3;
            }
            "begin" => self.env(out, pend),
            "end" => {
                self.raw_group();
            }
            "not" => {
                let a = self.arg();
                let s = match a.as_str() {
                    "=" => "≠".to_string(),
                    "∈" => "∉".to_string(),
                    "⊂" => "⊄".to_string(),
                    "⊃" => "⊅".to_string(),
                    "⊆" => "⊈".to_string(),
                    "⊇" => "⊉".to_string(),
                    "≡" => "≢".to_string(),
                    "<" => "≮".to_string(),
                    ">" => "≯".to_string(),
                    "∼" => "≁".to_string(),
                    "≈" => "≉".to_string(),
                    _ => format!("{}\u{338}", a),
                };
                rel(out, pend, &s);
            }
            "overset" | "stackrel" | "underset" => {
                let a = self.arg();
                let b = self.arg();
                let s = format!("{}{}", b, script(&a, n != "underset"));
                emit(out, pend, &s);
            }
            "xrightarrow" | "xleftarrow" => {
                self.skip_bracket_opt();
                let a = self.arg();
                let s = if a.is_empty() {
                    if n == "xrightarrow" {
                        "→".to_string()
                    } else {
                        "←".to_string()
                    }
                } else if n == "xrightarrow" {
                    format!("—{}→", a)
                } else {
                    format!("←{}—", a)
                };
                rel(out, pend, &s);
            }
            "substack" => {
                let a = self.arg().replace('\n', ", ");
                emit(out, pend, &a);
            }
            "tag" => {
                if let Some(raw) = self.raw_group() {
                    out.push_str(&format!("  ({})", self.text_of(&raw)));
                }
            }
            "hspace" => {
                self.raw_group();
                out.push(' ');
            }
            "textcolor" | "colorbox" => {
                self.raw_group();
                let a = self.arg();
                emit(out, pend, &a);
            }
            "color" => {
                self.raw_group();
            }
            "boxed" | "fbox" => {
                let a = self.arg();
                let s = format!("[{}]", a);
                emit(out, pend, &s);
            }
            "pmod" | "pod" => {
                let a = self.arg();
                flush(out, pend, '(');
                sp(out);
                out.push_str(&if n == "pmod" { format!("(mod {})", a) } else { format!("({})", a) });
            }
            "bmod" | "mod" => rel(out, pend, "mod"),
            "quad" => out.push('\u{2003}'),
            "qquad" => out.push_str("\u{2003}\u{2003}"),
            "enspace" => out.push('\u{2002}'),
            "thinspace" => out.push('\u{2009}'),
            "space" | "hfill" => out.push(' '),
            "newline" => {
                out.push(if self.display { '\n' } else { ' ' });
                *pend = 0;
            }
            "bf" | "bfseries" => self.style = Style::Bold,
            "rm" | "normalfont" | "sf" | "tt" => self.style = Style::Upright,
            "it" | "itshape" => self.style = Style::Italic,
            "cal" => self.style = Style::Cal,
            _ => self.table_cmd(n, out, pend),
        }
    }

    fn table_cmd(&mut self, n: &str, out: &mut String, pend: &mut u8) {
        if let Some(s) = lookup(GREEK, n) {
            emit(out, pend, s);
        } else if let Some(s) = lookup(RELS, n) {
            rel(out, pend, s);
        } else if let Some(s) = lookup(BINS, n) {
            bin(out, pend, s);
        } else if let Some(s) = lookup(ORDS, n) {
            emit(out, pend, s);
        } else if let Some(s) = lookup(OPS, n) {
            emit(out, pend, s);
            *pend = 3;
        } else if FUNCS.contains(&n) {
            emit(out, pend, n);
            *pend = 2;
        } else if let Some((_, comb)) = ACCENTS.iter().find(|(k, _)| *k == n) {
            let a = self.arg();
            let s = accent(&a, *comb);
            emit(out, pend, &s);
        } else if DELIMS.contains(&n) {
            self.skip_ws();
            if self.peek() == Some('.') {
                self.p += 1;
            }
        } else if DROP.contains(&n) {
        } else if DROP1.contains(&n) {
            self.raw_group();
        } else if let Some(st) = font_of(n) {
            let a = self.with_style(st);
            emit(out, pend, &a);
        } else if PASS.contains(&n) {
            let a = self.arg();
            emit(out, pend, &a);
        } else {
            let s = format!("\\{}", n);
            emit(out, pend, &s);
            *pend = 3;
        }
    }

    fn env(&mut self, out: &mut String, pend: &mut u8) {
        let name = self.raw_group().unwrap_or_default();
        let base = name.trim_end_matches('*').to_string();
        if matches!(base.as_str(), "array" | "tabular" | "alignedat" | "subarray") {
            self.raw_group();
        }
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut row: Vec<String> = Vec::new();
        loop {
            self.in_env += 1;
            let cell = self.seq(false, true);
            self.in_env -= 1;
            row.push(cell.trim_matches(' ').to_string());
            match self.peek() {
                Some('&') => self.p += 1,
                Some('\\') if self.peek_at(1) == Some('\\') => {
                    self.p += 2;
                    self.skip_bracket_opt();
                    rows.push(std::mem::take(&mut row));
                }
                Some('\\') => {
                    self.p += 4;
                    self.raw_group();
                    rows.push(std::mem::take(&mut row));
                    break;
                }
                _ => {
                    rows.push(std::mem::take(&mut row));
                    break;
                }
            }
        }
        if rows.len() > 1 && rows.last().map_or(false, |r| r.iter().all(|c| c.is_empty())) {
            rows.pop();
        }
        let is_mat = base.ends_with("matrix");
        let is_cases = matches!(base.as_str(), "cases" | "dcases");
        let (l, r) = match base.as_str() {
            "pmatrix" => ("(", ")"),
            "bmatrix" => ("[", "]"),
            "Bmatrix" => ("{", "}"),
            "vmatrix" => ("|", "|"),
            "Vmatrix" => ("‖", "‖"),
            _ => ("", ""),
        };
        let tabular = is_mat || is_cases || base == "array" || base == "tabular";
        let sep = if tabular { "  " } else { " " };
        let lines: Vec<String> = rows
            .iter()
            .map(|r| {
                let cells: Vec<&str> = r.iter().map(|s| s.as_str()).filter(|s| is_mat || !s.is_empty()).collect();
                cells.join(sep)
            })
            .collect();
        let s = if self.display {
            let mut v = Vec::with_capacity(lines.len());
            for (i, ln) in lines.iter().enumerate() {
                if is_mat && !l.is_empty() {
                    v.push(format!("{}{}{}", l, ln, r));
                } else if is_cases {
                    v.push(format!("{}{}", if i == 0 { "{ " } else { "  " }, ln));
                } else {
                    v.push(ln.clone());
                }
            }
            v.join("\n")
        } else {
            let j = lines.join("; ");
            if is_mat && !l.is_empty() {
                format!("{}{}{}", l, j, r)
            } else if is_cases {
                format!("{{ {}", j)
            } else {
                j
            }
        };
        emit(out, pend, &s);
        *pend = 0;
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn i(s: &str) -> String {
        latex_to_unicode(s, false)
    }
    fn d(s: &str) -> String {
        latex_to_unicode(s, true)
    }

    #[test]
    fn basics() {
        assert_eq!(i("E = mc^2"), "𝐸 = 𝑚𝑐²");
        assert_eq!(i("E=mc^2"), "𝐸 = 𝑚𝑐²");
        assert_eq!(i("h"), "ℎ");
        assert_eq!(i("x + 1"), "𝑥 + 1");
        assert_eq!(i("-x"), "−𝑥");
        assert_eq!(i("x = -1"), "𝑥 = −1");
        assert_eq!(i("f(x, y)"), "𝑓(𝑥, 𝑦)");
        assert_eq!(i("1{,}000"), "1,000");
        assert_eq!(i("f'(x)"), "𝑓′(𝑥)");
        assert_eq!(i(""), "");
    }

    #[test]
    fn greek() {
        assert_eq!(i(r"\alpha + \beta"), "α + β");
        assert_eq!(i(r"\Gamma \Omega \varepsilon \vartheta \varphi"), "ΓΩεϑφ");
        assert_eq!(i(r"\theta_1"), "θ₁");
    }

    #[test]
    fn functions() {
        assert_eq!(i(r"\sin x"), "sin 𝑥");
        assert_eq!(i(r"\sin(x)"), "sin(𝑥)");
        assert_eq!(i(r"\sin^2 x + \cos^2 x = 1"), "sin² 𝑥 + cos² 𝑥 = 1");
        assert_eq!(i(r"\lim_{n \to \infty} a_n"), "lim_(𝑛 → ∞) 𝑎ₙ");
        assert_eq!(i(r"\operatorname{softmax}(x)"), "softmax(𝑥)");
        assert_eq!(i(r"\log x"), "log 𝑥");
        assert_eq!(i(r"\max_i x_i"), "maxᵢ 𝑥ᵢ");
    }

    #[test]
    fn scripts() {
        assert_eq!(i("x^{n+1}"), "𝑥ⁿ⁺¹");
        assert_eq!(i("a_{ij}"), "𝑎ᵢⱼ");
        assert_eq!(i("x^2"), "𝑥²");
        assert_eq!(i("x^{-1}"), "𝑥⁻¹");
        assert_eq!(i(r"x^q"), "𝑥^𝑞");
        assert_eq!(i(r"x^{q+1}"), "𝑥^(𝑞 + 1)");
        assert_eq!(i(r"a_{q}"), "𝑎_𝑞");
        assert_eq!(i(r"x_i^2"), "𝑥ᵢ²");
        assert_eq!(i(r"90^\circ"), "90°");
        assert_eq!(i(r"A^T"), "𝐴ᵀ");
        assert_eq!(i(r"x_{\text{max}}"), "𝑥ₘₐₓ");
        assert_eq!(i(r"\sum_{i=1}^{n} x_i"), "∑ᵢ₌₁ⁿ 𝑥ᵢ");
        assert_eq!(i(r"\int_0^1 f(x)\,dx"), "∫₀¹ 𝑓(𝑥)\u{2009}𝑑𝑥");
    }

    #[test]
    fn fractions() {
        assert_eq!(i(r"\frac{1}{2}"), "1/2");
        assert_eq!(i(r"\dfrac{a}{b}"), "𝑎/𝑏");
        assert_eq!(i(r"\frac{a+b}{c+d}"), "(𝑎 + 𝑏)/(𝑐 + 𝑑)");
        assert_eq!(i(r"\frac{x^2}{2}"), "𝑥²/2");
        assert_eq!(i(r"\frac{-1}{2}"), "−1/2");
        assert_eq!(i(r"\binom{n}{k}"), "C(𝑛,𝑘)");
        assert_eq!(i(r"\frac{(a+b)}{2}"), "(𝑎 + 𝑏)/2");
    }

    #[test]
    fn roots() {
        assert_eq!(i(r"\sqrt{x}"), "√𝑥");
        assert_eq!(i(r"\sqrt{x+1}"), "√(𝑥 + 1)");
        assert_eq!(i(r"\sqrt[3]{x}"), "∛𝑥");
        assert_eq!(i(r"\sqrt[4]{x}"), "∜𝑥");
        assert_eq!(i(r"\sqrt[n]{x+1}"), "ⁿ√(𝑥 + 1)");
        assert_eq!(i(r"\sqrt{(x+1)}"), "√(𝑥 + 1)");
    }

    #[test]
    fn operators() {
        assert_eq!(i(r"a \cdot b \times c"), "𝑎 · 𝑏 × 𝑐");
        assert_eq!(i(r"a \le b"), "𝑎 ≤ 𝑏");
        assert_eq!(i(r"a<=b"), "𝑎 ≤ 𝑏");
        assert_eq!(i(r"f: A \to B"), "𝑓: 𝐴 → 𝐵");
        assert_eq!(i(r"x \in \mathbb{R}"), "𝑥 ∈ ℝ");
        assert_eq!(i(r"x \notin A \cup B"), "𝑥 ∉ 𝐴 ∪ 𝐵");
        assert_eq!(i(r"\forall x \exists y"), "∀𝑥∃𝑦");
        assert_eq!(i(r"P \Rightarrow Q"), "𝑃 ⇒ 𝑄");
        assert_eq!(i(r"a \iff b"), "𝑎 ⟺ 𝑏");
        assert_eq!(i(r"1, 2, \ldots, n"), "1, 2, …, 𝑛");
        assert_eq!(i(r"\nabla \cdot F"), "∇ · 𝐹");
        assert_eq!(i(r"a \not= b"), "𝑎 ≠ 𝑏");
        assert_eq!(i(r"a \neq b"), "𝑎 ≠ 𝑏");
        assert_eq!(i(r"\partial f"), "∂𝑓");
        assert_eq!(i(r"a \pm b"), "𝑎 ± 𝑏");
        assert_eq!(i(r"a \sim b"), "𝑎 ∼ 𝑏");
    }

    #[test]
    fn fonts() {
        assert_eq!(i(r"\mathbb{R}^n"), "ℝⁿ");
        assert_eq!(i(r"\mathbb{N}\mathbb{Z}\mathbb{Q}\mathbb{C}"), "ℕℤℚℂ");
        assert_eq!(i(r"\mathbf{x}"), "𝐱");
        assert_eq!(i(r"\mathbf{Ax} = \mathbf{b}"), "𝐀𝐱 = 𝐛");
        assert_eq!(i(r"\mathcal{L}"), "ℒ");
        assert_eq!(i(r"\mathcal{N}(0,1)"), "𝒩(0, 1)");
        assert_eq!(i(r"\mathfrak{g}"), "𝔤");
        assert_eq!(i(r"\mathrm{d}x"), "d𝑥");
        assert_eq!(i(r"\mathit{ab}"), "𝑎𝑏");
        assert_eq!(i(r"\text{if } x > 0"), "if 𝑥 > 0");
        assert_eq!(i(r"x \text{ and } y"), "𝑥 and 𝑦");
        assert_eq!(i(r"\textbf{bold}"), "bold");
        assert_eq!(i(r"\mathbb{1}"), "𝟙");
    }

    #[test]
    fn accents() {
        assert_eq!(i(r"\hat{x}"), "𝑥\u{302}");
        assert_eq!(i(r"\bar{x}"), "𝑥\u{304}");
        assert_eq!(i(r"\vec{v}"), "𝑣\u{20D7}");
        assert_eq!(i(r"\dot{x} \ddot{x}"), "𝑥\u{307}𝑥\u{308}");
        assert_eq!(i(r"\tilde x"), "𝑥\u{303}");
        assert_eq!(i(r"\overline{AB}"), "𝐴\u{305}𝐵\u{305}");
        assert_eq!(i(r"\underline{x}"), "𝑥\u{332}");
    }

    #[test]
    fn delimiters() {
        assert_eq!(i(r"\left( x \right)"), "(𝑥)");
        assert_eq!(i(r"\left\{ x \right\}"), "{𝑥}");
        assert_eq!(i(r"\left. x \right|"), "𝑥|");
        assert_eq!(i(r"\Big( x \Big)"), "(𝑥)");
        assert_eq!(i(r"\langle x, y \rangle"), "⟨𝑥, 𝑦⟩");
        assert_eq!(i(r"\lfloor x \rfloor"), "⌊𝑥⌋");
        assert_eq!(i(r"\lceil x \rceil"), "⌈𝑥⌉");
        assert_eq!(i(r"\| x \|"), "‖𝑥‖");
        assert_eq!(i(r"\left| x \right|"), "|𝑥|");
        assert_eq!(i(r"\left\langle x \right\rangle"), "⟨𝑥⟩");
        assert_eq!(i(r"\bigl( x \bigr)"), "(𝑥)");
    }

    #[test]
    fn spacing() {
        assert_eq!(i(r"a\,b"), "𝑎\u{2009}𝑏");
        assert_eq!(i(r"a\;b"), "𝑎\u{2004}𝑏");
        assert_eq!(i(r"a\quad b"), "𝑎\u{2003}𝑏");
        assert_eq!(i(r"a\qquad b"), "𝑎\u{2003}\u{2003}𝑏");
        assert_eq!(i(r"a~b"), "𝑎\u{a0}𝑏");
        assert_eq!(i(r"a\!b"), "𝑎𝑏");
        assert_eq!(i("a   b \n c"), "𝑎𝑏𝑐");
        assert_eq!(i(r"\%\$\&\#\_"), "%$&#_");
        assert_eq!(i(r"\displaystyle \sum\limits_{i} a"), "∑ᵢ 𝑎");
    }

    #[test]
    fn environments() {
        assert_eq!(d(r"\begin{aligned} a &= b \\ c &= d \end{aligned}"), "𝑎 = 𝑏\n𝑐 = 𝑑");
        assert_eq!(d(r"\begin{align*} x &= 1 \\ y &= 2 \\ \end{align*}"), "𝑥 = 1\n𝑦 = 2");
        assert_eq!(d(r"\begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}"), "(1  2)\n(3  4)");
        assert_eq!(i(r"\begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}"), "(1  2; 3  4)");
        assert_eq!(d(r"\begin{bmatrix} a & b \end{bmatrix}"), "[𝑎  𝑏]");
        assert_eq!(d(r"\begin{vmatrix} a & b \\ c & d \end{vmatrix}"), "|𝑎  𝑏|\n|𝑐  𝑑|");
        assert_eq!(d(r"\begin{matrix} a & b \end{matrix}"), "𝑎  𝑏");
        assert_eq!(d(r"f(x) = \begin{cases} 1 & x > 0 \\ 0 & \text{otherwise} \end{cases}"), "𝑓(𝑥) = { 1  𝑥 > 0\n  0  otherwise");
        assert_eq!(d(r"\begin{array}{cc} a & b \\ c & d \end{array}"), "𝑎  𝑏\n𝑐  𝑑");
        assert_eq!(d(r"\begin{gather} a \\ b \end{gather}"), "𝑎\n𝑏");
        assert_eq!(i(r"\begin{cases} a & b \\ c & d \end{cases}"), "{ 𝑎  𝑏; 𝑐  𝑑");
    }

    #[test]
    fn misc_commands() {
        assert_eq!(i(r"\boxed{x}"), "[𝑥]");
        assert_eq!(i(r"\color{red}{x}"), "𝑥");
        assert_eq!(i(r"\textcolor{red}{x} y"), "𝑥𝑦");
        assert_eq!(i(r"{\color{blue} x}"), "𝑥");
        assert_eq!(i(r"a \pmod{n}"), "𝑎 (mod 𝑛)");
        assert_eq!(i(r"\xrightarrow{f} "), "—𝑓→");
        assert_eq!(i(r"\overset{!}{=}"), "=^!");
        assert_eq!(i(r"x \tag{1}"), "𝑥  (1)");
        assert_eq!(i(r"x \label{eq:1}"), "𝑥");
    }

    #[test]
    fn unknown_and_unbalanced() {
        assert_eq!(i(r"\foo"), r"\foo");
        assert_eq!(i(r"\foo x"), r"\foo 𝑥");
        assert_eq!(i("{x"), "𝑥");
        assert_eq!(i("x}"), "𝑥}");
        assert_eq!(i(r"x\"), r"𝑥\");
        assert_eq!(i(r"\frac{1}"), "1/");
        assert_eq!(i("{{{"), "");
        assert_eq!(i(r"\begin{aligned} a"), "𝑎");
        assert_eq!(i(r"x^"), "𝑥");
    }

    #[test]
    fn display_newlines() {
        assert_eq!(d(r"a \\ b"), "𝑎\n𝑏");
        assert_eq!(i(r"a \\ b"), "𝑎; 𝑏");
    }

    #[test]
    fn realistic() {
        assert_eq!(d(r"\int_{-\infty}^{\infty} e^{-x^2} \, dx = \sqrt{\pi}"), "∫_(−∞)^∞ 𝑒^(−𝑥²)\u{2009}𝑑𝑥 = √π");
        let s = d(r"\mathbb{E}[X] = \sum_{k=0}^{\infty} k \, P(X = k)");
        assert!(s.starts_with("𝔼[𝑋] = ∑"), "{}", s);
        let s = i(r"\nabla_\theta J(\theta) = \frac{1}{N}\sum_{i=1}^N \nabla_\theta \ell_i");
        assert!(s == "∇_θ𝐽(θ) = 1/𝑁∑ᵢ₌₁ᴺ ∇_θℓᵢ", "{}", s);
    }

    fn lcg(seed: &mut u64) -> usize {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 33) as usize
    }

    #[test]
    fn fuzz_prefixes() {
        let forms = [
            r"\int_{-\infty}^{\infty} e^{-x^2} \, dx = \sqrt{\pi}",
            r"\begin{aligned} a &= \frac{b+c}{d} \\ \sqrt[3]{x} &\le \left( \sum_{i=1}^n x_i \right) \end{aligned}",
            r"f(x) = \begin{cases} 1 & x > 0 \\ \text{otherwise} & \mathbb{R} \end{cases}",
            r"\begin{pmatrix} \hat{a} & \vec{b} \\ \overline{c} & \binom{n}{k} \end{pmatrix}",
            r"\textcolor{red}{\mathbf{x}} \not\in \operatorname{span}\{\alpha_1, \ldots\} \xrightarrow[a]{b} $x$ é ∑",
        ];
        for f in forms.iter() {
            let idx: Vec<usize> = f.char_indices().map(|(k, _)| k).chain(std::iter::once(f.len())).collect();
            for &k in &idx {
                let _ = i(&f[..k]);
                let _ = d(&f[..k]);
                let _ = d(&f[k..]);
            }
            let _ = d(f);
        }
    }

    #[test]
    fn fuzz_garbage() {
        let frags = [
            "\\",
            "{",
            "}",
            "^",
            "_",
            "&",
            "\\\\",
            "\\frac",
            "\\begin{",
            "\\end{",
            "[",
            "]",
            "$",
            "x",
            " ",
            "\\sqrt",
            "\\left",
            "\\right",
            "\\text{",
            "\\begin{pmatrix}",
            "\\end{pmatrix}",
            "\\hat",
            "\\not",
            "\\sum",
            "é",
            "😀",
            "'",
            "-",
            "=",
            "\\mathbb",
            "\\color",
            "\\\\[",
            "\\operatorname",
            "\\xrightarrow",
            "\\overset",
            "\\tag",
            "*",
            "%",
            "\u{338}",
        ];
        let mut seed = 12345u64;
        for _ in 0..4000 {
            let n = lcg(&mut seed) % 30;
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(frags[lcg(&mut seed) % frags.len()]);
            }
            let _ = i(&s);
            let _ = d(&s);
        }
    }

    #[test]
    fn deep_and_big_inputs() {
        let big = "{".repeat(100_000);
        let _ = i(&big);
        let big = r"\frac".repeat(20_000);
        let _ = i(&big);
        let big = "^".repeat(50_000);
        let _ = i(&big);
        let big = r"\text{".repeat(20_000);
        let _ = i(&big);
        let big = r"\sqrt[".repeat(20_000);
        let _ = i(&big);
        let big = r"\begin{a}".repeat(20_000);
        let _ = d(&big);
        let big = r"\text{$".repeat(5_000);
        let _ = d(&big);
        let big = "x_{i} + ".repeat(50_000);
        let t = std::time::Instant::now();
        let _ = i(&big);
        assert!(t.elapsed().as_secs() < 3);
    }
}
