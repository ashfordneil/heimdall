use crate::error::Error;

bitflags::bitflags! {
    /// A set of possible types of tokens.
    pub struct TokenSet: u8 {
        const NEGATE = 1 << 0;
        const SEPARATOR = 1 << 1;
        const STAR = 1 << 2;
        const QUESTION = 1 << 3;
        const SQUARE_START = 1 << 4;
        const SQUARE_END = 1 << 5;
        const DASH = 1 << 6;
        const LITERAL = 1 << 7;
    }
}

impl TokenSet {
    fn test_char(self, target: char) -> Option<Token> {
        match target {
            '!' if self.contains(TokenSet::NEGATE) => Some(Token::Negate),
            '/' if self.contains(TokenSet::SEPARATOR) => Some(Token::Separator),
            '*' if self.contains(TokenSet::STAR) => Some(Token::Star),
            '?' if self.contains(TokenSet::QUESTION) => Some(Token::Question),
            '[' if self.contains(TokenSet::SQUARE_START) => Some(Token::SquareStart),
            ']' if self.contains(TokenSet::SQUARE_END) => Some(Token::SquareEnd),
            '-' if self.contains(TokenSet::DASH) => Some(Token::Dash),
            _ => None,
        }
    }
}

/// A token pulled from the parser.
pub enum Token {
    Ending,
    Negate,
    Separator,
    Star,
    Question,
    SquareStart,
    SquareEnd,
    Dash,
}

pub struct Tokenizer<'a> {
    inner: &'a str,
    index: usize,
    last_index: usize,
}

impl<'a> Tokenizer<'a> {
    /// Create a new tokenizer
    pub fn new(inner: &'a str) -> Self {
        Tokenizer {
            inner,
            index: 0,
            last_index: 0,
        }
    }

    fn remaining(&self) -> &'a str {
        &self.inner[self.index..]
    }

    pub fn flush(&mut self) {
        self.last_index = self.index;
    }

    pub fn reset(&mut self) {
        self.index = self.last_index;
    }

    /// Take a token from the start of the target, that fits into the accepted token set. An empty
    /// token set matches the end of the string.
    pub fn next_token(&mut self, accepted: TokenSet) -> Option<Token> {
        let remaining = self.remaining();

        if accepted == TokenSet::empty() {
            if remaining == "" {
                return Some(Token::Ending);
            } else {
                return None;
            }
        }

        let output = remaining
            .chars()
            .next()
            .and_then(|letter| accepted.test_char(letter));

        if output.is_some() {
            self.index += 1;
        }

        output
    }

    /// Take a string literal from the target, that is terminated by any one of the tokens in the
    /// follow set.
    pub fn read_literal(&mut self, follow: TokenSet) -> Option<&'a str> {
        let index = self
            .remaining()
            .find(|letter| follow.test_char(letter).is_some());

        if let Some(index) = index {
            let (start, _remaining) = self.remaining().split_at(index);
            if start == "" {
                None
            } else {
                self.index += index;
                Some(start)
            }
        } else {
            let rest = self.remaining();
            self.index = self.inner.len();
            if rest == "" {
                None
            } else {
                Some(rest)
            }
        }
    }

    /// Create an error message from the current position.
    pub fn error(&self, token_set: TokenSet) -> Error {
        Error::InvalidGlobParse(self.inner.to_string(), token_set, self.index)
    }
}
