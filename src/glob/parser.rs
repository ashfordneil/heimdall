use crate::{
    error::Result,
    glob::tokenizer::{Token, TokenSet, Tokenizer},
};

use regex::Regex;
use regex_syntax::hir::{self, Hir};
use std::iter;

#[derive(Debug)]
pub struct Ast {
    pub starts_negated: bool,
    pub segments: Vec<Segment>,
}

#[derive(Debug)]
pub enum Segment {
    Pattern(Regex),
    Anything,
    Separator,
}

fn question() -> Hir {
    let mut class = hir::ClassUnicode::new(iter::once(hir::ClassUnicodeRange::new('/', '/')));
    class.negate();
    Hir::class(hir::Class::Unicode(class))
}

fn star() -> Hir {
    Hir::repetition(hir::Repetition {
        kind: hir::RepetitionKind::ZeroOrMore,
        greedy: true,
        hir: Box::new(question()),
    })
}

fn parse_charset(tokens: &mut Tokenizer) -> Result<Hir> {
    let mut class = hir::ClassUnicode::empty();

    let text = tokens
        .read_literal(TokenSet::SQUARE_END)
        .ok_or(tokens.error(TokenSet::SQUARE_END))?;

    let mut letters = text.chars().peekable();
    while let Some(letter) = letters.next() {
        if letters.peek() == Some(&'-') {
            letters.next();
            let other_letter = letters.next().ok_or(tokens.error(TokenSet::LITERAL))?;
            class.push(hir::ClassUnicodeRange::new(letter, other_letter));
        } else {
            class.push(hir::ClassUnicodeRange::new(letter, letter));
        }
    }

    let output = Hir::class(hir::Class::Unicode(class));
    Ok(output)
}

fn parse_pattern(tokens: &mut Tokenizer) -> Result<Option<Regex>> {
    let mut constructor = Vec::new();

    let accept_set = TokenSet::STAR | TokenSet::QUESTION | TokenSet::SQUARE_START;
    let break_set = accept_set | TokenSet::SEPARATOR;

    loop {
        match tokens.next_token(accept_set) {
            Some(Token::Star) => {
                if tokens.next_token(TokenSet::STAR).is_some() {
                    tokens.reset();
                    break;
                }
                constructor.push(star());
            }
            Some(Token::Question) => constructor.push(question()),
            Some(Token::SquareStart) => constructor.push(parse_charset(tokens)?),
            Some(_) => unreachable!(),
            None => match tokens.read_literal(break_set) {
                Some(literal) => {
                    let letters = literal
                        .chars()
                        .map(|letter| Hir::literal(hir::Literal::Unicode(letter)));
                    constructor.extend(letters);
                }
                None => break,
            },
        }

        tokens.flush();
    }

    if constructor.is_empty() {
        Ok(None)
    } else {
        let total = Hir::concat(constructor);
        let string = format!("^{}$", total);
        Ok(Some(Regex::new(&string).unwrap()))
    }
}

fn parse_segment(tokens: &mut Tokenizer) -> Result<Option<Segment>> {
    if let Some(regex) = parse_pattern(tokens)? {
        return Ok(Some(Segment::Pattern(regex)));
    }

    let output = tokens
        .next_token(TokenSet::STAR | TokenSet::SEPARATOR)
        .map(|token| match token {
            Token::Star => {
                // The only reason we'd be getting a star here if parse_pattern failed is if there
                // are two stars in a row
                tokens.next_token(TokenSet::STAR).unwrap();
                Segment::Anything
            }
            Token::Separator => Segment::Separator,
            _ => unreachable!(),
        });

    if output.is_some() {
        tokens.flush();
    }

    Ok(output)
}

pub fn parse(input: &str) -> Result<Ast> {
    let mut tokens = Tokenizer::new(input);
    let starts_negated = tokens.next_token(TokenSet::NEGATE).is_some();

    let mut segments = Vec::new();

    while let Some(segment) = parse_segment(&mut tokens)? {
        segments.push(segment);
    }

    if let Some(Token::Ending) = tokens.next_token(TokenSet::empty()) {
        Ok(Ast {
            starts_negated,
            segments,
        })
    } else {
        Err(tokens.error(TokenSet::empty()))
    }
}

#[cfg(test)]
mod test {
    use super::{parse, Segment};

    #[test]
    fn single_file() {
        let glob = parse("filename.txt").unwrap();
        assert_eq!(false, glob.starts_negated);
        let regex = match &glob.segments[..] {
            [Segment::Pattern(regex)] => regex,
            other => panic!("Incorrect pattern: {:?}", other),
        };
        assert_eq!(r"^filename\.txt$", regex.as_str());
    }

    #[test]
    fn negated_single_file() {
        let glob = parse("!.gitignore").unwrap();
        assert_eq!(true, glob.starts_negated);
        let regex = match &glob.segments[..] {
            [Segment::Pattern(regex)] => regex,
            other => panic!("Incorrect pattern: {:?}", other),
        };
        assert_eq!(r"^\.gitignore$", regex.as_str());
    }

    #[test]
    fn regular_path() {
        let glob = parse("path/to/file.txt").unwrap();
        assert_eq!(false, glob.starts_negated);
        let (path, to, file) = match &glob.segments[..] {
            [Segment::Pattern(path), Segment::Separator, Segment::Pattern(to), Segment::Separator, Segment::Pattern(file)] => {
                (path, to, file)
            }
            other => panic!("Incorrect pattern: {:?}", other),
        };
        assert_eq!("^path$", path.as_str());
        assert_eq!("^to$", to.as_str());
        assert_eq!(r"^file\.txt$", file.as_str());
    }

    #[test]
    fn has_question_mark() {
        let glob = parse("hello.?pp").unwrap();
        assert_eq!(false, glob.starts_negated);
        let regex = match &glob.segments[..] {
            [Segment::Pattern(regex)] => regex,
            other => panic!("Incorrect pattern: {:?}", other),
        };
        assert!(regex.is_match("hello.cpp"));
        assert!(regex.is_match("hello.hpp"));
        assert!(regex.is_match("hello.🚀pp"));
        assert!(!regex.is_match("hello./pp"));
        assert!(!regex.is_match("unrelated string"));
    }

    #[test]
    fn has_star() {
        let glob = parse("*.rs").unwrap();
        assert_eq!(false, glob.starts_negated);
        let regex = match &glob.segments[..] {
            [Segment::Pattern(regex)] => regex,
            other => panic!("Incorrect pattern: {:?}", other),
        };
        assert!(regex.is_match("main.rs"));
        assert!(regex.is_match("testing.rs"));
        assert!(!regex.is_match("path/to/file.rs"));
        assert!(!regex.is_match("unrelated string"));
    }

    #[test]
    fn has_starstar() {
        let glob = parse("target/**").unwrap();
        assert_eq!(false, glob.starts_negated);
        let regex = match &glob.segments[..] {
            [Segment::Pattern(regex), Segment::Separator, Segment::Anything] => regex,
            other => panic!("Incorrect pattern: {:?}", other),
        };
        assert_eq!("^target$", regex.as_str());
    }
}
