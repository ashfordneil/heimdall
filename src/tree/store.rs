use crate::fs::File;
use std::{
    hash::{BuildHasher, Hash, Hasher},
    os::unix::io::{AsRawFd, RawFd},
};

use ahash::RandomState;
use hashbrown::raw::RawTable;
use slab::Slab;

/// An entry into the tree.
#[derive(Debug, PartialEq, Eq)]
pub struct TreeEntry {
    fd: File,
    inode: u64,
}

impl TreeEntry {
    /// Create a new entry into the tree.
    pub fn new(fd: File, inode: u64) -> Self {
        TreeEntry { fd, inode }
    }

    pub fn fd(&self) -> &File {
        &self.fd
    }

    pub fn inode(&self) -> u64 {
        self.inode
    }
}

/// Indexed storage for the inside of the tree.
pub struct TreeStore {
    storage: Slab<TreeEntry>,
    fd_index: (RawTable<usize>, RandomState),
    inode_index: (RawTable<usize>, RandomState),
}

impl TreeStore {
    /// Create a new tree store
    pub fn new() -> Self {
        let storage = Slab::new();
        let fd_index = (RawTable::new(), RandomState::new());
        let inode_index = (RawTable::new(), RandomState::new());

        TreeStore {
            storage,
            fd_index,
            inode_index,
        }
    }

    fn hash<T: Hash>(hash_builder: &RandomState, data: &T) -> u64 {
        let mut hasher = hash_builder.build_hasher();
        data.hash(&mut hasher);
        hasher.finish()
    }

    /// Insert a new entry into the tree store. Returns an index that is used to uniquely identify
    /// that entry later. This function will panic if either the fd of the entry, or the inode, has
    /// been used before in a different entry. Reinserting an entry that is identical in all fields
    /// to an entry that has previously been inserted will return the existing key.
    pub fn insert(&mut self, entry: TreeEntry) -> usize {
        let TreeStore {
            storage,
            fd_index: (fd_table, fd_hasher),
            inode_index: (inode_table, inode_hasher),
        } = self;
        let fd_hash = Self::hash(fd_hasher, &entry.fd.as_raw_fd());
        let inode_hash = Self::hash(inode_hasher, &entry.inode);

        let fd_bucket = fd_table
            .find(fd_hash, |&index| storage[index] == entry)
            .map(|bucket| unsafe { bucket.read() });
        let inode_bucket = inode_table
            .find(inode_hash, |&index| storage[index] == entry)
            .map(|bucket| unsafe { bucket.read() });

        match (fd_bucket, inode_bucket) {
            (Some(fd_bucket), Some(inode_bucket)) if fd_bucket == inode_bucket => fd_bucket,
            (None, None) => {
                let key = storage.insert(entry);
                fd_table.insert(fd_hash, key, |&key2| {
                    Self::hash(fd_hasher, &storage[key2].fd.as_raw_fd())
                });
                inode_table.insert(inode_hash, key, |&key2| {
                    Self::hash(inode_hasher, &storage[key2].inode)
                });
                key
            }
            _ => unreachable!("Attempt to create entry that shares data with existing entry"),
        }
    }

    /// Lookup a tree entry by the key that it was stored with originally.
    pub fn key_to_entry(&self, key: usize) -> Option<&TreeEntry> {
        self.storage.get(key)
    }

    /// Lookup a tree key by the fd of the entry that it was stored with originally.
    pub fn fd_to_key(&self, fd: RawFd) -> Option<usize> {
        let (table, hasher) = &self.fd_index;
        let hash = Self::hash(hasher, &fd);
        table
            .find(hash, |&index| self.storage[index].fd.as_raw_fd() == fd)
            .map(|bucket| unsafe { bucket.read() })
    }

    /// Lookup a tree entry by its fd.
    pub fn fd_to_entry(&self, fd: RawFd) -> Option<&TreeEntry> {
        self.fd_to_key(fd).and_then(|key| self.key_to_entry(key))
    }

    /// Lookup a tree key by the inode of the entry that it was stored with originally.
    pub fn inode_to_key(&self, inode: u64) -> Option<usize> {
        let (table, hasher) = &self.inode_index;
        let hash = Self::hash(hasher, &inode);
        table
            .find(hash, |&index| self.storage[index].inode == inode)
            .map(|bucket| unsafe { bucket.read() })
    }

    /// Lookup a tree entry by its inode.
    pub fn inode_to_entry(&self, inode: u64) -> Option<&TreeEntry> {
        self.inode_to_key(inode)
            .and_then(|key| self.key_to_entry(key))
    }
}
