//! Small, dependency-free syntax highlighter for fenced code blocks.
//!
//! The lexer is intentionally simple: comments, strings, numbers, identifiers
//! (classified as keyword, type, function or plain) and punctuation. It covers
//! the languages the app is likely to document without pulling in `syntect`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    pub kind: Kind,
}

struct Lang {
    keywords: &'static [&'static str],
    line_comments: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
}

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while", "Some", "None", "Ok", "Err",
];

const PYTHON: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];

const JS: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default",
    "delete", "do", "else", "export", "extends", "false", "finally", "for", "from", "function", "if",
    "import", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch",
    "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "yield",
];

const GO: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for",
    "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select",
    "struct", "switch", "type", "var", "nil", "true", "false",
];

const C: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum",
    "extern", "float", "for", "goto", "if", "inline", "int", "long", "namespace", "return", "short",
    "signed", "sizeof", "static", "struct", "switch", "template", "typedef", "union", "unsigned",
    "using", "void", "volatile", "while", "class", "public", "private", "protected", "new", "delete",
    "true", "false", "nullptr", "constexpr", "auto",
];

const SWIFT: &[&str] = &[
    "associatedtype", "class", "deinit", "enum", "extension", "func", "import", "init", "inout",
    "internal", "let", "open", "operator", "private", "protocol", "public", "static", "struct",
    "subscript", "typealias", "var", "break", "case", "continue", "default", "defer", "do", "else",
    "fallthrough", "for", "guard", "if", "in", "repeat", "return", "switch", "where", "while", "as",
    "catch", "false", "is", "nil", "rethrows", "super", "self", "Self", "throw", "throws", "true",
    "try",
];

const BASH: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "function", "in", "select", "time", "return", "exit", "export", "local", "readonly", "declare",
    "set", "unset", "shift", "source", "alias", "echo", "cd", "true", "false",
];

const SQL: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create",
    "table", "alter", "drop", "index", "join", "left", "right", "inner", "outer", "on", "group",
    "by", "order", "having", "limit", "offset", "as", "and", "or", "not", "null", "primary", "key",
    "foreign", "references", "distinct", "union", "all", "case", "when", "then", "end", "asc",
    "desc", "default", "constraint",
];

const TOML: &[&str] = &["true", "false"];

fn lang_for(name: &str) -> Option<Lang> {
    let name = name
        .trim()
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
        .next()
        .unwrap_or("")
        .to_owned();

    let lang = match name.as_str() {
        "rust" | "rs" => Lang {
            keywords: RUST,
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"'],
        },
        "python" | "py" => Lang {
            keywords: PYTHON,
            line_comments: &["#"],
            block: None,
            quotes: &['"', '\''],
        },
        "js" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" => Lang {
            keywords: JS,
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
        },
        "go" | "golang" => Lang {
            keywords: GO,
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"', '`'],
        },
        "c" | "h" | "cpp" | "c++" | "cc" | "hpp" | "objc" | "java" | "csharp" | "cs" => Lang {
            keywords: C,
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"'],
        },
        "swift" => Lang {
            keywords: SWIFT,
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"'],
        },
        "bash" | "sh" | "zsh" | "shell" | "console" => Lang {
            keywords: BASH,
            line_comments: &["#"],
            block: None,
            quotes: &['"', '\''],
        },
        "sql" => Lang {
            keywords: SQL,
            line_comments: &["--"],
            block: Some(("/*", "*/")),
            quotes: &['"', '\''],
        },
        "toml" | "ini" => Lang {
            keywords: TOML,
            line_comments: &["#"],
            block: None,
            quotes: &['"', '\''],
        },
        "yaml" | "yml" => Lang {
            keywords: TOML,
            line_comments: &["#"],
            block: None,
            quotes: &['"', '\''],
        },
        "json" => Lang {
            keywords: TOML,
            line_comments: &[],
            block: None,
            quotes: &['"'],
        },
        "html" | "xml" | "svg" => Lang {
            keywords: &[],
            line_comments: &[],
            block: Some(("<!--", "-->")),
            quotes: &['"', '\''],
        },
        "css" | "scss" => Lang {
            keywords: &[],
            line_comments: &[],
            block: Some(("/*", "*/")),
            quotes: &['"', '\''],
        },
        _ => return None,
    };
    Some(lang)
}

pub fn tokens(lang: &str, src: &str) -> Vec<Token> {
    let Some(lang) = lang_for(lang) else {
        return vec![Token {
            text: src.to_owned(),
            kind: Kind::Plain,
        }];
    };

    let chars: Vec<char> = src.chars().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;

    while i < chars.len() {
        if lang
            .line_comments
            .iter()
            .any(|prefix| starts_with(&chars, i, prefix))
        {
            flush(&mut out, &mut plain, Kind::Plain);
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            push(&mut out, chars[start..i].iter().collect(), Kind::Comment);
            continue;
        }

        if let Some((open, close)) = lang.block {
            if starts_with(&chars, i, open) {
                flush(&mut out, &mut plain, Kind::Plain);
                let start = i;
                i += open.len();
                while i < chars.len() && !starts_with(&chars, i, close) {
                    i += 1;
                }
                i = (i + close.len()).min(chars.len());
                push(&mut out, chars[start..i].iter().collect(), Kind::Comment);
                continue;
            }
        }

        let c = chars[i];
        if lang.quotes.contains(&c) {
            flush(&mut out, &mut plain, Kind::Plain);
            let start = i;
            if is_triple(&chars, i, c) {
                i += 3;
                while i < chars.len() && !is_triple(&chars, i, c) {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 3).min(chars.len());
            } else {
                i += 1;
                while i < chars.len() && chars[i] != c && chars[i] != '\n' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    i += 1;
                }
                if i < chars.len() && chars[i] == c {
                    i += 1;
                }
            }
            push(&mut out, chars[start..i].iter().collect(), Kind::String);
            continue;
        }

        if c.is_ascii_digit() {
            flush(&mut out, &mut plain, Kind::Plain);
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                if chars[i] == '.' && i + 1 < chars.len() && !chars[i + 1].is_ascii_digit() {
                    break;
                }
                i += 1;
            }
            push(&mut out, chars[start..i].iter().collect(), Kind::Number);
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            flush(&mut out, &mut plain, Kind::Plain);
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let kind = classify(&word, &chars, i, &lang);
            push(&mut out, word, kind);
            continue;
        }

        plain.push(c);
        i += 1;
    }

    flush(&mut out, &mut plain, Kind::Plain);
    out
}

fn classify(word: &str, chars: &[char], at: usize, lang: &Lang) -> Kind {
    if lang.keywords.contains(&word) {
        return Kind::Keyword;
    }
    if matches!(word, "true" | "false" | "null" | "nil" | "None" | "True" | "False") {
        return Kind::Keyword;
    }
    let mut j = at;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    if j < chars.len() && chars[j] == '(' {
        return Kind::Function;
    }
    if word.chars().next().is_some_and(|c| c.is_uppercase()) {
        return Kind::Type;
    }
    Kind::Plain
}

fn starts_with(chars: &[char], at: usize, needle: &str) -> bool {
    let mut j = at;
    for c in needle.chars() {
        if chars.get(j) != Some(&c) {
            return false;
        }
        j += 1;
    }
    true
}

fn is_triple(chars: &[char], at: usize, quote: char) -> bool {
    chars.get(at) == Some(&quote)
        && chars.get(at + 1) == Some(&quote)
        && chars.get(at + 2) == Some(&quote)
}

fn push(out: &mut Vec<Token>, text: String, kind: Kind) {
    if let Some(last) = out.last_mut() {
        if last.kind == kind {
            last.text.push_str(&text);
            return;
        }
    }
    out.push(Token { text, kind });
}

fn flush(out: &mut Vec<Token>, plain: &mut String, kind: Kind) {
    if !plain.is_empty() {
        push(out, std::mem::take(plain), kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: &str, src: &str) -> Vec<Kind> {
        tokens(lang, src).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn rust_spans() {
        let t = tokens("rust", "// c\nfn main() { let x = \"s\"; }");
        assert!(t.iter().any(|t| t.kind == Kind::Comment && t.text == "// c"));
        assert!(t.iter().any(|t| t.kind == Kind::Keyword && t.text == "fn"));
        assert!(t.iter().any(|t| t.kind == Kind::String && t.text == "\"s\""));
        assert!(kinds("rust", "1 + 2").contains(&Kind::Number));
    }

    #[test]
    fn unknown_language_is_plain() {
        let t = tokens("nonsense", "fn main() {}");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].kind, Kind::Plain);
    }

    #[test]
    fn text_is_preserved() {
        let src = "fn a() { /* x */ }\n// y\nlet s = \"z\";\n";
        let joined: String = tokens("rust", src).into_iter().map(|t| t.text).collect();
        assert_eq!(joined, src);
    }
}
