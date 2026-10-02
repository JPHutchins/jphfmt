use logos::{Lexer, Logos};

/// The lexical category of a [`Token`]. Trivia (whitespace, newlines, comments) are
/// first-class kinds, not skipped, so the token stream is lossless: concatenating every
/// token's text reproduces the source exactly.
/// A literal's escape: `\` and whatever follows it, **including a newline**. `\\.` would not do —
/// the regex crate's `.` matches `\r` and not `\n`, so a `\`-continued literal was one token under
/// CRLF and several under LF, and normalizing the endings (§2.1) then handed the next pass a
/// different tokenization (#110). C splices a `\`-newline in translation phase 2, before
/// tokenization, so either flavour is one literal.
///
/// A subpattern rather than the same fragment twice, because a string and a character literal must
/// never disagree about what an escape is.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(subpattern escape = r"\\[\s\S]")]
pub enum TokenKind {
    #[regex(r"\r\n|\r|\n")]
    Newline,
    #[regex(r"[ \t\x0C\x0B]+")]
    Whitespace,
    #[token("//", lex_line_comment)]
    LineComment,
    #[token("/*", lex_block_comment)]
    BlockComment,
    // Not a widening in kind: `[^"\\]` already matches a bare newline, so a literal holding one is
    // already one token. Only the escaped newline was read two ways.
    #[regex(r#""([^"\\]|(?&escape))*""#)]
    String,
    #[regex(r"'([^'\\]|(?&escape))*'")]
    Char,
    // C11 §6.4.8's pp-number: an exponent's sign belongs to the number, so `1e-5` and `0x1p-1022`
    // are one token and no later pass reads that `-` as an operator to space.
    #[regex(r"[0-9]([eEpP][-+]|[0-9a-zA-Z._'])*")]
    #[regex(r"\.[0-9]([eEpP][-+]|[0-9a-zA-Z._'])*")]
    Number,
    #[regex(r"[A-Za-z_][A-Za-z0-9_]*")]
    Ident,
    #[token("...")]
    #[token("<<=")]
    #[token(">>=")]
    #[token("->")]
    #[token("++")]
    #[token("--")]
    #[token("<<")]
    #[token(">>")]
    #[token("<=")]
    #[token(">=")]
    #[token("==")]
    #[token("!=")]
    #[token("&&")]
    #[token("||")]
    #[token("+=")]
    #[token("-=")]
    #[token("*=")]
    #[token("/=")]
    #[token("%=")]
    #[token("&=")]
    #[token("|=")]
    #[token("^=")]
    #[token("##")]
    Operator,
    #[regex(r"[-+*/%&|^~!<>=?:;,.()\[\]{}#\\@]")]
    Punct,
    /// Never matched by the lexer; assigned to any byte logos fails to classify so the
    /// stream stays lossless (see [`tokenize`]).
    Unknown,
}

/// Extend a `//` match to just before its logical line's end (the newline stays its own token). A `\`
/// ending a physical line continues the comment onto the next: phase 2 splices lines before phase 3
/// removes comments, so the line after is the comment's text, not code. Blanks between the `\` and
/// the line end still splice, as GCC and Clang read them. The blanks are whatever `str::trim_end`
/// removes, because that is what the trailing-blank trim writes away: a narrower reading would lex
/// the author's line as code and the trimmed one as comment, and change between passes.
fn lex_line_comment(lex: &mut Lexer<TokenKind>) {
    let rem = lex.remainder();
    let line_end = |from: usize| rem[from..].find(['\n', '\r']).map(|i| from + i);
    let mut end = 0;
    while let Some(break_at) = line_end(end)
        && splices(&rem[end..break_at])
    {
        end = break_at
            + if rem[break_at..].starts_with("\r\n") {
                2
            } else {
                1
            };
    }
    lex.bump(line_end(end).unwrap_or(rem.len()));
}

/// Whether a physical line ends in a splice: a `\` with nothing after it but blanks, read as
/// [`lex_line_comment`] reads them. Phase 2 deletes a backslash-newline whatever precedes the `\`,
/// so a line ending in `\\` splices as one ending in `\` does (#190). The one reading of the
/// question for every pass that asks it of text.
pub(crate) fn splices(line: &str) -> bool {
    line.trim_end().ends_with('\\')
}

/// Extend a `/*` match to the closing `*/`, or to end-of-input if unterminated.
fn lex_block_comment(lex: &mut Lexer<TokenKind>) {
    let rem = lex.remainder();
    lex.bump(rem.find("*/").map_or(rem.len(), |i| i + 2));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'src> {
    pub kind: TokenKind,
    pub text: &'src str,
}

/// Lex `src` into a lossless token stream: `tokenize(src).iter().map(|t| t.text).collect::<String>()`
/// equals `src` for every input. A byte logos cannot classify becomes a [`TokenKind::Unknown`]
/// token carrying that slice rather than being dropped.
pub fn tokenize(src: &str) -> Vec<Token<'_>> {
    let mut lex = TokenKind::lexer(src);
    let mut out = Vec::new();
    while let Some(result) = lex.next() {
        out.push(Token {
            kind: result.unwrap_or(TokenKind::Unknown),
            text: lex.slice(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_line_comment_captures_comment_text() {
        // Comment with newline: captures "// simple" (stops before newline)
        let mut lex = TokenKind::lexer("// simple\n");
        assert_eq!(lex.next(), Some(Ok(TokenKind::LineComment)));
        assert_eq!(lex.slice(), "// simple");

        // Comment at end of input (no newline): captures to end
        let mut lex = TokenKind::lexer("// no newline");
        assert_eq!(lex.next(), Some(Ok(TokenKind::LineComment)));
        assert_eq!(lex.slice(), "// no newline");

        // Empty comment: captures just "//"
        let mut lex = TokenKind::lexer("//\n");
        assert_eq!(lex.next(), Some(Ok(TokenKind::LineComment)));
        assert_eq!(lex.slice(), "//");
    }

    #[test]
    fn splices_reads_a_backslash_ending_the_line() {
        assert!(splices("foo \\"));
        assert!(splices("#define M(a) ((a) + 1) \\"));
        assert!(splices("foo \\ \t"));
        assert!(splices("int x; // c \\\\"));
        assert!(!splices("foo bar"));
        assert!(!splices(""));
        assert!(!splices("foo \\ x"));
    }

    #[test]
    fn lex_line_comment_continues_across_a_splice() {
        for (src, comment) in [
            ("// c \\\nif (a)\n{", "// c \\\nif (a)"),
            ("// c \\   \nif (a)\n{", "// c \\   \nif (a)"),
            ("// c \\\x0C\nif (a)\n{", "// c \\\x0C\nif (a)"),
            ("// c \\\x0B\t\nif (a)\n{", "// c \\\x0B\t\nif (a)"),
            ("// c \\\u{a0}\nif (a)\n{", "// c \\\u{a0}\nif (a)"),
            ("// c \\\r\nx\r\ny", "// c \\\r\nx"),
            ("// a \\\nb \\\nc\nd", "// a \\\nb \\\nc"),
            ("// c \\", "// c \\"),
            ("// c \\ x\ny", "// c \\ x"),
        ] {
            let mut lex = TokenKind::lexer(src);
            assert_eq!(lex.next(), Some(Ok(TokenKind::LineComment)), "{src:?}");
            assert_eq!(lex.slice(), comment, "{src:?}");
        }
    }

    #[test]
    fn lex_line_comment_then_newline_and_more() {
        let mut lex = TokenKind::lexer("// comment\nnext");
        assert_eq!(lex.next(), Some(Ok(TokenKind::LineComment)));
        assert_eq!(lex.slice(), "// comment");
        assert_eq!(lex.next(), Some(Ok(TokenKind::Newline)));
        assert_eq!(lex.slice(), "\n");
        assert_eq!(lex.next(), Some(Ok(TokenKind::Ident)));
        assert_eq!(lex.slice(), "next");
    }

    #[test]
    fn lex_number_takes_an_exponent_sign() {
        // C11 §6.4.8: `e+`, `e-`, `p+`, `p-` continue a pp-number. Without them the sign lexes as
        // an operator, and a later pass spaces it into `1e - 5`, which does not compile.
        // The unsigned forms take the other branch of the alternation, and are the common case.
        for src in [
            "1e-5",
            "1.0e+10",
            "0x1p-1022",
            "0x1.62066151add8bp+10",
            "0x1E-2",
            "1e5",
            "1E10",
            "0x1p10",
            "1.0e10",
        ] {
            let mut lex = TokenKind::lexer(src);
            assert_eq!(lex.next(), Some(Ok(TokenKind::Number)), "{src}");
            assert_eq!(lex.slice(), src);
        }
    }

    #[test]
    fn lex_number_leaves_a_bare_minus_alone() {
        // Only an exponent's sign joins the number: `1-2` is still three tokens.
        let mut lex = TokenKind::lexer("1-2");
        assert_eq!(lex.next(), Some(Ok(TokenKind::Number)));
        assert_eq!(lex.slice(), "1");
        assert_eq!(lex.next(), Some(Ok(TokenKind::Punct)));
        assert_eq!(lex.slice(), "-");
        assert_eq!(lex.next(), Some(Ok(TokenKind::Number)));
        assert_eq!(lex.slice(), "2");
        assert_eq!(lex.next(), None);
    }
}
