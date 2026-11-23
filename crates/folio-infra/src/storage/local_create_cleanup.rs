use std::path::Path;

use rustix::fs::{open, openat, unlinkat, AtFlags, Dir, Mode, OFlags};
use rustix::io::{Errno, Result};

pub(super) fn clean(
    root: &Path,
    key: &object_store::path::Path,
    discard_object: bool,
) -> Result<()> {
    let mut parts: Vec<_> = key.parts().collect();
    let name = parts.pop().ok_or(Errno::INVAL)?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let mut directory = open(root, flags, Mode::empty())?;
    for part in parts {
        directory = match openat(&directory, part.as_ref(), flags, Mode::empty()) {
            Ok(child) => child,
            Err(Errno::NOENT) => return Ok(()),
            Err(error) => return Err(error),
        };
    }

    // LocalFileSystem 0.14.2 appends #<digits> before linking a create and
    // filters those names from list(). A successful link can leave its stage
    // behind if unlink fails, so remove those names before declaring success.
    let name = name.as_ref();
    let staging_prefix = format!("{name}#");
    for entry in Dir::read_from(&directory)? {
        let entry = entry?;
        let entry_name = entry.file_name();
        if entry_name
            .to_bytes()
            .strip_prefix(staging_prefix.as_bytes())
            .is_some_and(|suffix| !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit))
        {
            match unlinkat(&directory, entry_name, AtFlags::empty()) {
                Ok(()) | Err(Errno::NOENT) => {}
                Err(error) => return Err(error),
            }
        }
    }
    if discard_object {
        // A writer that linked the object just before we unlinked its stage
        // may have left the final name too.
        match unlinkat(&directory, name, AtFlags::empty()) {
            Ok(()) | Err(Errno::NOENT) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
