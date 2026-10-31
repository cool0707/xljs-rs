use std::path::{Path, PathBuf};

use crate::{Error, Result, err};

/// Verifies that the paths exist.
pub(crate) fn check_paths(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Err(err!("shell.noFiles"));
    }
    for p in paths {
        std::fs::metadata(p).map_err(|e| Error::path("stat", p.clone(), e))?;
    }
    Ok(())
}

/// Absolute paths grouped by parent folder, keeping the order in which
/// folders first appear.
pub(crate) fn group_by_dir(paths: &[String]) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut out: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for p in paths {
        let abs = std::path::absolute(Path::new(p)).unwrap_or_else(|_| PathBuf::from(p));
        let dir = abs.parent().map(Path::to_path_buf).unwrap_or_default();
        match out.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, v)) => v.push(abs),
            None => out.push((dir, vec![abs])),
        }
    }
    out
}

/// Opens the file manager on the folders of the files (Explorer selects
/// them; macOS too; other systems just open the folder).
pub fn shell_show_in_folder(paths: &[String]) -> Result<()> {
    check_paths(paths)?;
    #[cfg(windows)]
    {
        super::native::show_in_folder(&group_by_dir(paths))
    }
    #[cfg(not(windows))]
    {
        for (dir, files) in group_by_dir(paths) {
            let mut cmd = if cfg!(target_os = "macos") {
                let mut c = std::process::Command::new("open");
                c.arg("-R").args(&files);
                c
            } else {
                let mut c = std::process::Command::new("xdg-open");
                c.arg(&dir);
                c
            };
            cmd.spawn().map_err(|e| Error::msg(e.to_string()))?;
        }
        Ok(())
    }
}

/// Opens a file, folder or URL with its default application.
pub fn shell_open(target: &str) -> Result<()> {
    #[cfg(windows)]
    {
        super::native::shell_execute(target)
    }
    #[cfg(not(windows))]
    {
        let name = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        std::process::Command::new(name).arg(target).spawn().map_err(|e| Error::msg(e.to_string()))?;
        Ok(())
    }
}
