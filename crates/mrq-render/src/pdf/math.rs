//! A small LaTeX-to-Typst math converter for the common subset: letters, numbers and
//! operators, `^`/`_` attachments, groups, Greek letters and symbols, named operators
//! (`\sin`, `\lim`, …), `\frac`, `\sqrt`, `\binom`, font commands (`\mathbb`, `\mathbf`,
//! …), accents (`\hat`, `\vec`, …), `\text` and `\left`/`\right`. Anything else
//! (environments, `&`, `\\`, unknown commands, unbalanced groups) is not converted.

/// `tex` (without its `$` delimiters) as Typst math, or `None` when it uses something
/// the converter does not know. The result has no `$` delimiters either.
pub fn tex_to_typst(tex: &str) -> Option<String> {
    let mut parser = Parser {
        chars: tex.chars().collect(),
        pos: 0,
    };
    let items = parser.sequence(false)?;
    (parser.pos == parser.chars.len()).then(|| items.join(" "))
}

/// Typst symbols for LaTeX commands without arguments.
const SYMBOLS: &[(&str, &str)] = &[
    // Greek letters; LaTeX's `\epsilon` and `\phi` are Typst's `.alt` forms.
    ("alpha", "alpha"),
    ("beta", "beta"),
    ("gamma", "gamma"),
    ("delta", "delta"),
    ("epsilon", "epsilon.alt"),
    ("varepsilon", "epsilon"),
    ("zeta", "zeta"),
    ("eta", "eta"),
    ("theta", "theta"),
    ("vartheta", "theta.alt"),
    ("iota", "iota"),
    ("kappa", "kappa"),
    ("lambda", "lambda"),
    ("mu", "mu"),
    ("nu", "nu"),
    ("xi", "xi"),
    ("pi", "pi"),
    ("varpi", "pi.alt"),
    ("rho", "rho"),
    ("varrho", "rho.alt"),
    ("sigma", "sigma"),
    ("varsigma", "sigma.alt"),
    ("tau", "tau"),
    ("upsilon", "upsilon"),
    ("phi", "phi.alt"),
    ("varphi", "phi"),
    ("chi", "chi"),
    ("psi", "psi"),
    ("omega", "omega"),
    ("Gamma", "Gamma"),
    ("Delta", "Delta"),
    ("Theta", "Theta"),
    ("Lambda", "Lambda"),
    ("Xi", "Xi"),
    ("Pi", "Pi"),
    ("Sigma", "Sigma"),
    ("Upsilon", "Upsilon"),
    ("Phi", "Phi"),
    ("Psi", "Psi"),
    ("Omega", "Omega"),
    // Big operators.
    ("sum", "sum"),
    ("prod", "product"),
    ("coprod", "product.co"),
    ("int", "integral"),
    ("iint", "integral.double"),
    ("iiint", "integral.triple"),
    ("oint", "integral.cont"),
    ("bigcup", "union.big"),
    ("bigcap", "inter.big"),
    // Relations and binary operators.
    ("cdot", "dot.op"),
    ("times", "times"),
    ("div", "div"),
    ("pm", "plus.minus"),
    ("mp", "minus.plus"),
    ("ast", "ast.op"),
    ("star", "star.op"),
    ("circ", "compose"),
    ("bullet", "bullet"),
    ("le", "<="),
    ("leq", "<="),
    ("ge", ">="),
    ("geq", ">="),
    ("ne", "!="),
    ("neq", "!="),
    ("ll", "<<"),
    ("gg", ">>"),
    ("approx", "approx"),
    ("equiv", "equiv"),
    ("sim", "tilde.op"),
    ("simeq", "tilde.eq"),
    ("cong", "tilde.equiv"),
    ("propto", "prop"),
    ("in", "in"),
    ("notin", "in.not"),
    ("ni", "in.rev"),
    ("subset", "subset"),
    ("subseteq", "subset.eq"),
    ("supset", "supset"),
    ("supseteq", "supset.eq"),
    ("cup", "union"),
    ("cap", "inter"),
    ("setminus", "without"),
    ("wedge", "and"),
    ("land", "and"),
    ("vee", "or"),
    ("lor", "or"),
    ("neg", "not"),
    ("lnot", "not"),
    ("forall", "forall"),
    ("exists", "exists"),
    ("mid", "divides"),
    ("parallel", "parallel"),
    ("perp", "perp"),
    // Arrows.
    ("to", "->"),
    ("rightarrow", "->"),
    ("leftarrow", "<-"),
    ("gets", "<-"),
    ("leftrightarrow", "<->"),
    ("Rightarrow", "=>"),
    ("implies", "=>"),
    ("Leftarrow", "arrow.l.double"),
    ("Leftrightarrow", "<=>"),
    ("iff", "<=>"),
    ("mapsto", "|->"),
    ("uparrow", "arrow.t"),
    ("downarrow", "arrow.b"),
    // Other symbols.
    ("infty", "infinity"),
    ("partial", "partial"),
    ("nabla", "nabla"),
    ("emptyset", "emptyset"),
    ("varnothing", "emptyset"),
    ("ell", "ell"),
    ("hbar", "planck"),
    ("prime", "prime"),
    ("angle", "angle"),
    ("degree", "degree"),
    ("ldots", "dots.h"),
    ("dots", "dots.h"),
    ("cdots", "dots.c"),
    ("vdots", "dots.v"),
    ("ddots", "dots.down"),
    ("langle", "chevron.l"),
    ("rangle", "chevron.r"),
    ("lfloor", "floor.l"),
    ("rfloor", "floor.r"),
    ("lceil", "ceil.l"),
    ("rceil", "ceil.r"),
    ("lvert", "|"),
    ("rvert", "|"),
    ("vert", "|"),
    ("lVert", "||"),
    ("rVert", "||"),
    ("Vert", "||"),
    // Spacing.
    ("quad", "quad"),
    ("qquad", "wide"),
    // Named operators, which Typst has under the same name.
    ("sin", "sin"),
    ("cos", "cos"),
    ("tan", "tan"),
    ("cot", "cot"),
    ("sec", "sec"),
    ("csc", "csc"),
    ("arcsin", "arcsin"),
    ("arccos", "arccos"),
    ("arctan", "arctan"),
    ("sinh", "sinh"),
    ("cosh", "cosh"),
    ("tanh", "tanh"),
    ("log", "log"),
    ("ln", "ln"),
    ("lg", "lg"),
    ("exp", "exp"),
    ("lim", "lim"),
    ("liminf", "liminf"),
    ("limsup", "limsup"),
    ("max", "max"),
    ("min", "min"),
    ("sup", "sup"),
    ("inf", "inf"),
    ("det", "det"),
    ("gcd", "gcd"),
    ("deg", "deg"),
    ("dim", "dim"),
    ("ker", "ker"),
    ("arg", "arg"),
    ("Pr", "Pr"),
];

/// Commands with one argument and the Typst function they become.
const FUNCTIONS: &[(&str, &str)] = &[
    ("sqrt", "sqrt"),
    ("mathbb", "bb"),
    ("mathbf", "bold"),
    ("boldsymbol", "bold"),
    ("bm", "bold"),
    ("mathit", "italic"),
    ("mathrm", "upright"),
    ("mathcal", "cal"),
    ("mathfrak", "frak"),
    ("mathsf", "sans"),
    ("mathtt", "mono"),
    ("hat", "hat"),
    ("widehat", "hat"),
    ("bar", "macron"),
    ("overline", "overline"),
    ("underline", "underline"),
    ("vec", "arrow"),
    ("dot", "dot"),
    ("ddot", "dot.double"),
    ("tilde", "tilde"),
    ("widetilde", "tilde"),
    ("overbrace", "overbrace"),
    ("underbrace", "underbrace"),
];

/// Commands whose argument is text.
const TEXT: &[&str] = &["text", "textrm", "textit", "textbf", "mbox", "operatorname"];

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    /// Items up to the end, or up to the closing `}` of a group (`in_group`).
    fn sequence(&mut self, in_group: bool) -> Option<Vec<String>> {
        let mut items: Vec<String> = Vec::new();
        loop {
            self.skip_space();
            let Some(c) = self.peek() else {
                return (!in_group).then_some(items);
            };
            match c {
                '}' if in_group => return Some(items),
                '^' | '_' => {
                    self.pos += 1;
                    let arg = self.argument()?;
                    let base = items.pop().unwrap_or_else(|| "\"\"".to_owned());
                    items.push(format!("{base}{c}({arg})"));
                }
                _ => {
                    let item = self.atom()?;
                    if !item.is_empty() {
                        items.push(item);
                    }
                }
            }
        }
    }

    /// One item; empty for something that shows nothing (`\!`, `\left.`).
    fn atom(&mut self) -> Option<String> {
        let c = self.peek()?;
        self.pos += 1;
        let item = match c {
            '{' => {
                let inner = self.sequence(true)?;
                self.pos += 1;
                inner.join(" ")
            }
            '\\' => return self.command(),
            '0'..='9' => {
                let mut number = c.to_string();
                while let Some(next) = self.peek() {
                    let digit_after_dot = next == '.'
                        && self
                            .chars
                            .get(self.pos + 1)
                            .is_some_and(char::is_ascii_digit);
                    if next.is_ascii_digit() || digit_after_dot {
                        number.push(next);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                number
            }
            '~' => "space".to_owned(),
            // A slash is a fraction in Typst.
            '/' => "\\/".to_owned(),
            '+' | '-' | '=' | '<' | '>' | '(' | ')' | '[' | ']' | ',' | ';' | ':' | '!' | '|'
            | '\'' | '.' | '*' | '?' => c.to_string(),
            '}' | '&' | '#' | '$' | '%' | '"' | '@' | '`' => return None,
            c if c.is_alphabetic() => c.to_string(),
            c if c.is_control() => return None,
            // Other symbols (`α`, `≤`, …) mean themselves.
            c => c.to_string(),
        };
        Some(item)
    }

    /// After a `\`: a command and its arguments.
    fn command(&mut self) -> Option<String> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        if self.pos == start {
            // A control symbol.
            let c = self.peek()?;
            self.pos += 1;
            let item = match c {
                '{' | '}' | '$' | '#' | '&' | '_' => format!("\\{c}"),
                '%' => "%".to_owned(),
                '|' => "||".to_owned(),
                ',' => "thin".to_owned(),
                ':' | '>' => "med".to_owned(),
                ';' => "thick".to_owned(),
                ' ' => "space".to_owned(),
                '!' => String::new(),
                _ => return None,
            };
            return Some(item);
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        if let Some((_, symbol)) = SYMBOLS.iter().find(|(tex, _)| *tex == name) {
            return Some((*symbol).to_owned());
        }
        if let Some((_, function)) = FUNCTIONS.iter().find(|(tex, _)| *tex == name) {
            if name == "sqrt" {
                self.skip_space();
                if self.peek() == Some('[') {
                    self.pos += 1;
                    let index = self.until(']')?;
                    let index = tex_to_typst(&index)?;
                    let arg = self.argument()?;
                    return Some(format!("root({index}, {arg})"));
                }
            }
            let arg = self.argument()?;
            return Some(format!("{function}({arg})"));
        }
        if TEXT.contains(&name.as_str()) {
            self.skip_space();
            if self.peek() != Some('{') {
                return None;
            }
            self.pos += 1;
            let text = self.until('}')?;
            let string = format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
            return Some(if name == "operatorname" {
                format!("op({string})")
            } else {
                string
            });
        }
        match name.as_str() {
            "frac" | "dfrac" | "tfrac" | "binom" => {
                let a = self.argument()?;
                let b = self.argument()?;
                let function = if name == "binom" { "binom" } else { "frac" };
                Some(format!("{function}({a}, {b})"))
            }
            // Typst sizes delimiters by itself; `.` is no delimiter.
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl"
            | "Bigr" => {
                self.skip_space();
                if self.peek() == Some('.') {
                    self.pos += 1;
                    return Some(String::new());
                }
                self.atom()
            }
            "displaystyle" | "textstyle" | "limits" | "nolimits" => Some(String::new()),
            _ => None,
        }
    }

    /// The argument of a command or attachment: a group or a single item.
    fn argument(&mut self) -> Option<String> {
        self.skip_space();
        if self.peek() == Some('{') {
            self.pos += 1;
            let inner = self.sequence(true)?;
            self.pos += 1;
            return Some(inner.join(" "));
        }
        match self.peek()? {
            // A single digit, not a whole number: `x^23` is `x^(2) 3`.
            c @ '0'..='9' => {
                self.pos += 1;
                Some(c.to_string())
            }
            '}' | '^' | '_' => None,
            _ => self.atom(),
        }
    }

    /// The raw text up to `end` (balancing braces), consuming `end`.
    fn until(&mut self, end: char) -> Option<String> {
        let mut depth = 0usize;
        let mut text = String::new();
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '{' => depth += 1,
                '}' if depth > 0 => depth -= 1,
                c if c == end && depth == 0 => return Some(text),
                _ => {}
            }
            text.push(c);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fmt::Write as _;

    use super::{FUNCTIONS, SYMBOLS, TEXT, tex_to_typst};
    use crate::pdf::world::MrqWorld;

    /// Every command the converter knows compiles in Typst without errors or warnings
    /// (a renamed or deprecated Typst symbol would warn).
    #[test]
    fn every_known_command_compiles_cleanly() {
        let mut tex: Vec<String> = SYMBOLS
            .iter()
            .map(|(name, _)| format!("\\{name}"))
            .collect();
        tex.extend(FUNCTIONS.iter().map(|(name, _)| format!("\\{name}{{x}}")));
        tex.extend(TEXT.iter().map(|name| format!("\\{name}{{t}}")));
        tex.extend(
            [
                "\\frac{a}{b}",
                "\\binom{n}{k}",
                "\\sqrt[3]{x}",
                "\\left( x \\right.",
                "a\\,b\\;c\\:d\\!e\\ f",
                "\\{\\}\\|\\%\\$\\#\\&\\_",
                "x'' / y ~ z",
            ]
            .map(str::to_owned),
        );
        let mut source = String::new();
        for tex in &tex {
            let typst = tex_to_typst(tex).unwrap_or_else(|| panic!("{tex} converts"));
            let _ = write!(source, "${typst}$\n\n");
        }
        let world = MrqWorld::new(source, HashMap::new(), None, Vec::new(), None);
        let warned = typst::compile::<typst_layout::PagedDocument>(&world);
        let warnings: Vec<String> = warned
            .warnings
            .iter()
            .map(|w| w.message.to_string())
            .collect();
        assert!(warnings.is_empty(), "{warnings:?}");
        if let Err(errors) = warned.output {
            let errors: Vec<String> = errors.iter().map(|e| e.message.to_string()).collect();
            panic!("{errors:?}");
        }
    }
}
