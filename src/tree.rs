use self::{
    ignore::Ignore,
    store::{TreeEntry, TreeStore},
};
use crate::{
    fs::{File, FileType},
    graph::Graph,
    Result,
};
use std::collections::HashMap;
use std::{
    ffi::{CString, OsStr},
    fmt::{Debug, Formatter},
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

mod ignore;
mod store;

/// How one node in the tree is connected to another node in the tree.
#[derive(Debug)]
enum Connection {
    /// This node is a directory, and the connected node is one of it's entries.
    Child(CString),
    /// This node is a symbolic link, and the connected node is what it links to.
    SymLink,
}

/// An in-memory wrapper around a directory tree.
pub struct Tree {
    root_dir: PathBuf,
    root_entry: usize,
    storage: TreeStore,
    structure: Graph<Connection>,
    ignores: Ignore,
}

impl Tree {
    /// Open up a path, and create a tree at that location.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root_dir = root.as_ref().canonicalize()?;

        let mut output = Tree {
            root_dir,
            // fix this up soon, leave it as max_value so it's obvious there's an error in case it
            // isn't fixed up
            root_entry: usize::max_value(),
            storage: TreeStore::new(),
            structure: Graph::new(),
            ignores: Ignore::new(),
        };

        let (file_type, root_entry) = {
            let fd = {
                let path = CString::new(output.root_dir.as_os_str().as_bytes())
                    .expect("Canonicalized path contains nul byte");
                File::open(&path)?
            };
            let (file_type, inode) = fd.stat()?;
            (file_type, TreeEntry::new(fd, inode))
        };
        let mut unresolved_files = Vec::new();
        let mut unresolved_symlinks = Vec::new();

        output.root_entry = output.add_file(root_entry, file_type, &mut unresolved_files)?;

        while let Some(action) = unresolved_files.pop() {
            output.add_child_file(
                action.key,
                action.path,
                &mut unresolved_files,
                &mut unresolved_symlinks,
            )?;
        }

        for UnresolvedSymlink { key, path } in unresolved_symlinks {
            let parent_key = if let Some(edge) = output
                .structure
                .incoming(key)
                .find(|edge| edge.connects_to != key)
            {
                edge.connects_to
            } else {
                log::warn!("Symlink found without parent directory");
                continue;
            };
            let path = Path::new(OsStr::from_bytes(path.as_bytes()));
            if let Some(target_key) = output.follow_path(parent_key, path) {
                output
                    .structure
                    .add_edge(key, target_key, Connection::SymLink);
            }
        }

        Ok(output)
    }

    /// Takes a position in the graph, and a path along the graph, and returns the position that
    /// that path would lead to - if that path exists and is in the walked section of the tree.
    fn follow_path(&self, mut key: usize, path: &Path) -> Option<usize> {
        for segment in path.components() {
            match segment {
                Component::CurDir => continue,
                Component::ParentDir => {
                    let parent = self
                        .structure
                        .incoming(key)
                        .find(|edge| edge.connects_to != key);
                    match parent {
                        Some(parent) => key = parent.connects_to,
                        None => {
                            log::warn!("Symlink climbs too high");
                            return None;
                        }
                    }
                }
                Component::Normal(component) => {
                    let child = self.structure.outgoing(key).find(|edge| match edge.weight {
                        Connection::SymLink => true,
                        Connection::Child(name) => component.as_bytes() == name.as_bytes(),
                    });
                    match child {
                        Some(child) => key = child.connects_to,
                        None => {
                            log::warn!("Symlink component unresolved");
                            return None;
                        }
                    }
                }
                unsupported => {
                    log::warn!("Symlink contains invalid path segment: {:?}", unsupported);
                    return None;
                }
            }
        }

        Some(key)
    }

    /// Adds a file as a child of a related file. Pushes any followup work that arises in adding
    /// this file to unresolved_files and unresolved_symlinks.
    fn add_child_file(
        &mut self,
        parent_key: usize,
        path: CString,
        unresolved_files: &mut Vec<UnresolvedFile>,
        unresolved_symlinks: &mut Vec<UnresolvedSymlink>,
    ) -> Result<()> {
        let parent_fd = self.storage.key_to_entry(parent_key).unwrap().fd();
        let (file_type, inode) = parent_fd.stat_at(&path)?;

        if !self.ignores.should_open(
            parent_key,
            OsStr::from_bytes(path.as_bytes()),
            file_type == FileType::Directory,
        ) {
            return Ok(());
        }

        let real_name = if file_type == FileType::Link {
            Some(parent_fd.get_link_name(&path)?)
        } else {
            None
        };

        let child_key = if let Some(key) = self.storage.inode_to_key(inode) {
            key
        } else {
            let mut fd = File::open_at(parent_fd, &path)?;
            if (path.as_bytes() == b".gitignore" && file_type == FileType::Regular) {
                self.ignores.parse_gitignore(&mut fd, parent_key)?;
            }
            let entry = TreeEntry::new(fd, inode);
            self.add_file(entry, file_type, unresolved_files)?
        };

        self.ignores
            .open_at(parent_key, OsStr::from_bytes(path.as_bytes()), child_key);
        self.structure
            .add_edge(parent_key, child_key, Connection::Child(path));

        if let Some(real_name) = real_name {
            unresolved_symlinks.push(UnresolvedSymlink {
                key: parent_key,
                path: real_name,
            })
        }

        Ok(())
    }

    /// Adds a file, and then returns the ID of that file. Pushes any followup work that arises in
    /// adding this file to unresolved_files.
    fn add_file(
        &mut self,
        entry: TreeEntry,
        file_type: FileType,
        unresolved_files: &mut Vec<UnresolvedFile>,
    ) -> Result<usize> {
        let mut children = if file_type == FileType::Directory {
            entry.fd().scan()?
        } else {
            Vec::new()
        };
        let key = self.storage.insert(entry);

        children.sort_by_key(|name| name.as_bytes() == b".gitignore");

        for child_path in children {
            unresolved_files.push(UnresolvedFile {
                key,
                path: child_path,
            });
        }

        Ok(key)
    }
}

/// For use during construction
struct UnresolvedFile {
    key: usize,
    path: CString,
}

/// For use during construction.
struct UnresolvedSymlink {
    key: usize,
    path: CString,
}

impl Debug for Tree {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut path = self.root_dir.clone();
        let mut stack = vec![(self.root_entry, self.structure.outgoing(self.root_entry))];

        while let Some((last, iterator)) = stack.last_mut() {
            match iterator.next() {
                Some(edge) => match edge.weight {
                    Connection::SymLink => {
                        writeln!(f, "[{}] Symlink {:?} -> {}", last, path, edge.connects_to)?
                    }
                    Connection::Child(name) => {
                        path.push(OsStr::from_bytes(name.as_bytes()));
                        let next = edge.connects_to;
                        if next != *last {
                            writeln!(f, "[{}] File {:?}", next, path)?;
                            stack.push((next, self.structure.outgoing(next)));
                        }
                    }
                },
                None => {
                    path.pop();
                    stack.pop();
                }
            }
        }

        Ok(())
    }
}
