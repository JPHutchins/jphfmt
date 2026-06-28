use logos::{Lexer, Logos};

/// The lexical category of a [`Token`]. Trivia (whitespace, newlines, comments) are
/// first-class kinds, not skipped, so the token stream is lossless: concatenating every
/// token's text reproduces the source exactly.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    #[regex(r"\r\n|\r|\n")]
    Newline,
    #[regex(r"[ \t\x0C\x0B]+")]
    Whitespace,
    #[token("//", lex_line_comment)]
    LineComment,
    #[token("/*", lex_block_comment)]
    BlockComment,
    #[regex(r#""([^"\\]|\\.)*""#)]
    String,
    #[regex(r"'([^'\\]|\\.)*'")]
    Char,
    #[regex(r"[0-9][0-9a-zA-Z._']*")]
    #[regex(r"\.[0-9][0-9a-zA-Z._']*")]
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

/// Extend a `//` match to just before the line's end (the newline stays its own token).
fn lex_line_comment(lex: &mut Lexer<TokenKind>) {
    let rem = lex.remainder();
    lex.bump(rem.find(['\n', '\r']).unwrap_or(rem.len()));
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
    fn lex_line_comment_then_newline_and_more() {
        let mut lex = TokenKind::lexer("// comment\nnext");
        assert_eq!(lex.next(), Some(Ok(TokenKind::LineComment)));
        assert_eq!(lex.slice(), "// comment");
        assert_eq!(lex.next(), Some(Ok(TokenKind::Newline)));
        assert_eq!(lex.slice(), "\n");
        assert_eq!(lex.next(), Some(Ok(TokenKind::Ident)));
        assert_eq!(lex.slice(), "next");
    }
}
