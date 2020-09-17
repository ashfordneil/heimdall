use heimdall::{Result, Tree};
use std::{
    ffi::CString,
    io::{BufReader, Read},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

use structopt::StructOpt;

/// Directory watcher - nothing happens in your file system that this system doesn't see.
///
/// This program sits on top of a directory and its subdirectories, and tracks any changes that
/// occur to the files within them.
#[derive(StructOpt)]
struct Arguments {
    /// The root directory to watch (defaults to the current working directory)
    #[structopt(default_value = ".", long = "root")]
    root: PathBuf,
}

fn main() -> Result<()> {
    let args = Arguments::from_args();
    env_logger::init();

    let tree = Tree::new(args.root)?;
    println!("{:?}", tree);
    Ok(())
}
