use self::parser::{Ast, Segment};
pub use self::tokenizer::TokenSet;
use crate::error::{Error, Result};
use std::{ffi::OsStr, iter};

use either::Either;
use itertools::Itertools;
use regex::Regex;
use slab::Slab;
use std::collections::HashMap;

mod parser;
mod tokenizer;

/// A single segment of a glob, used to match against segments of a path.
struct Glob {
    segment: Option<Regex>,
    negated: bool,
    trailing_slash: bool,
    relative: bool,
}

/// A key that indexes into the GlobArena.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct GlobKey(usize);

/// An arena of glob segments.
pub struct GlobArena {
    // The regexes for each glob section - if they aren't ** sections
    storage: Slab<Glob>,
    // for each glob segment, tell me if a segment follows it in the pattern that it was parsed from
    children: HashMap<usize, usize>,
}

impl GlobArena {
    /// Create the new glob arena.
    pub fn new() -> Self {
        GlobArena {
            storage: Slab::new(),
            children: HashMap::new(),
        }
    }

    /// Compile a new glob. Returns, if the compilation is successful, a key by which to index into
    /// the glob.
    pub fn compile_glob(&mut self, glob: &str) -> Result<GlobKey> {
        let Ast {
            starts_negated,
            segments,
        } = parser::parse(glob)?;

        let (fixed_path, segments) = match &segments[..] {
            [Segment::Separator, ..] => (true, segments.into_iter().skip(1)),
            [rest @ ..] => (rest.len() > 2, segments.into_iter().skip(0)),
        };

        let mut segments = segments
            .batching(|it| {
                let start = it.next()?;
                let output = match start {
                    Segment::Separator => {
                        return Some(Err(Error::InvalidGlobCompile(
                            glob.to_string(),
                            "unexpected /",
                        )))
                    }
                    other_segment => {
                        let segment = if let Segment::Pattern(regex) = other_segment {
                            Some(regex)
                        } else {
                            None
                        };
                        let trailing_slash = match it.next() {
                            Some(Segment::Separator) => true,
                            None => false,
                            _ => {
                                return Some(Err(Error::InvalidGlobCompile(
                                    glob.to_string(),
                                    "/ needed between sections",
                                )))
                            }
                        };
                        (segment, trailing_slash)
                    }
                };

                Some(Ok(output))
            })
            .collect::<Result<Vec<_>>>()?;

        let mut first_key = None;
        let mut latest_key = None;

        for (segment, trailing) in segments {
            let glob = Glob {
                segment: segment,
                negated: starts_negated,
                trailing_slash: trailing,
                relative: !fixed_path,
            };
            let key = self.storage.insert(glob);
            first_key.get_or_insert(GlobKey(key));

            if let Some(old_key) = latest_key {
                self.children.insert(old_key, key);
            }
            latest_key = Some(key);
        }

        first_key.ok_or_else(|| Error::InvalidGlobCompile(glob.to_string(), "no glob segments"))
    }

    // Some(true) means that the glob explicitly matches this file. Some(false) means that the glob
    // explicitly matches this file, but was negated. None means that the glob did not match this
    // file.
    pub fn match_file(&self, GlobKey(key): GlobKey, name: &OsStr, is_dir: bool) -> Option<bool> {
        if self.children.get(&key).is_some() {
            return None;
        }
        let glob = &self.storage[key];
        let name = name.to_str()?;

        let is_match = match &glob.segment {
            Some(regex) => regex.is_match(name),
            None => true,
        };

        if is_match {
            Some(!glob.negated)
        } else {
            None
        }
    }

    /// Find the glob that can be used to match against the children of this file. Returns either
    /// None, or Some(an iterator over the glob keys). Note that the glob keys returned by this
    /// method may include the current glob key, in the case of globs that are either not fixed to
    /// and directory or globs that contain the ** pattern.
    pub fn match_dir(
        &self,
        GlobKey(key): GlobKey,
        name: &OsStr,
    ) -> Option<impl Iterator<Item = GlobKey>> {
        let glob = &self.storage[key];
        let name = name.to_str()?;

        let is_child_match = match &glob.segment {
            Some(regex) => regex.is_match(name),
            None => true,
        };
        let child_match = if is_child_match {
            self.children.get(&key).cloned().map(GlobKey)
        } else {
            None
        };

        let loop_match = if glob.relative || glob.segment.is_none() {
            Some(GlobKey(key))
        } else {
            None
        };

        if child_match.is_none() && loop_match.is_none() {
            return None;
        }

        let output = child_match.into_iter().chain(loop_match.into_iter());

        Some(output)
    }
}

#[cfg(test)]
mod test {
    use super::GlobArena;

    #[test]
    fn no_double_slash() {
        let mut arena = GlobArena::new();
        arena.compile_glob("path//to/file").unwrap_err();
    }

    #[test]
    fn path_to_file() {
        let mut arena = GlobArena::new();
        let top_key = arena.compile_glob("path/to/file.txt").unwrap();

        assert_eq!(None, arena.match_file(top_key, "path".as_ref(), true));
        let &mid_key = match &arena
            .match_dir(top_key, "path".as_ref())
            .unwrap()
            .collect::<Vec<_>>()[..]
        {
            [key] => key,
            other => panic!("Wrong number of keys: {:?}", other),
        };

        assert_eq!(None, arena.match_file(mid_key, "to".as_ref(), true));
        let &low_key = match &arena
            .match_dir(mid_key, "to".as_ref())
            .unwrap()
            .collect::<Vec<_>>()[..]
        {
            [key] => key,
            other => panic!("Wrong number of keys: {:?}", other),
        };

        assert_eq!(
            Some(true),
            arena.match_file(low_key, "file.txt".as_ref(), false)
        );
    }

    #[test]
    fn plain_filename() {
        let mut arena = GlobArena::new();
        let key = arena.compile_glob("file.txt").unwrap();

        assert_eq!(
            Some(true),
            arena.match_file(key, "file.txt".as_ref(), false)
        );

        match &arena
            .match_dir(key, "any file".as_ref())
            .unwrap()
            .collect::<Vec<_>>()[..]
        {
            [child_key] => assert_eq!(key, *child_key),
            other => panic!("Incorrect child keys: {:?}", other),
        }
    }

    #[test]
    fn uses_star_star() {
        let mut arena = GlobArena::new();
        let key = arena.compile_glob("**/index.js").unwrap();

        assert_eq!(None, arena.match_file(key, "index.js".as_ref(), false));
        let child_key = match &arena
            .match_dir(key, "any directory".as_ref())
            .unwrap()
            .collect::<Vec<_>>()[..]
        {
            [key_one, key_two] => {
                assert_eq!(key, *key_two);
                *key_one
            }
            other => panic!("Incorrect child keys: {:?}", other),
        };

        assert_eq!(
            Some(true),
            arena.match_file(child_key, "index.js".as_ref(), false)
        );
    }
}
