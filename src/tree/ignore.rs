use crate::{
    error::Result,
    fs::File,
    glob::{GlobArena, GlobKey},
};
use std::{
    collections::HashMap,
    ffi::OsStr,
    io::{BufRead, BufReader},
    os::unix::ffi::OsStrExt,
};

pub struct Ignore {
    arena: GlobArena,
    key_to_globs: HashMap<usize, Vec<GlobKey>>,
}

impl Ignore {
    pub fn new() -> Self {
        Ignore {
            arena: GlobArena::new(),
            key_to_globs: HashMap::new(),
        }
    }

    pub fn parse_gitignore(&mut self, fd: &mut File, at: usize) -> Result<()> {
        let mut new_globs = Vec::new();

        let read = BufReader::new(fd);
        for line in read.lines() {
            let line = line?;
            if line.starts_with('#') || line.trim_end().is_empty() {
                continue;
            }
            match self.arena.compile_glob(line.as_str().trim_end()) {
                Ok(key) => new_globs.push(key),
                Err(err) => log::warn!("Invalid line of glob: {}", err),
            }
        }

        self.key_to_globs
            .entry(at)
            .or_insert_with(Vec::new)
            .extend_from_slice(new_globs.as_ref());

        Ok(())
    }

    pub fn should_open(&self, parent: usize, name: &OsStr, is_dir: bool) -> bool {
        {
            let name = name.as_bytes();
            if name.starts_with(b".") && name != b".gitignore" {
                return false;
            }
        }
        self.key_to_globs
            .get(&parent)
            .into_iter()
            .flat_map(|globs| globs.iter())
            .cloned()
            // test the name against the globs
            .map(|glob| self.arena.match_file(glob, name, is_dir))
            // turn from "does the file match" to "should we open the file"
            .map(|opt| opt.map(|x| !x))
            .fold(None, |old, new| match (old, new) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (old, new) => old.or(new),
            })
            // if the gitignore doesn't mention the file, open it
            .unwrap_or(true)
    }

    pub fn open_at(&mut self, parent: usize, name: &OsStr, child: usize) {
        let new_globs = self
            .key_to_globs
            .get(&parent)
            .into_iter()
            .flat_map(|globs| globs.iter())
            .cloned()
            .filter_map(|glob| self.arena.match_dir(glob, name))
            .flatten()
            .collect::<Vec<_>>();

        self.key_to_globs
            .entry(child)
            .or_insert_with(Vec::new)
            .extend_from_slice(new_globs.as_ref())
    }
}
