use crate::{ErrorKind, Result};
use anyhow::{Context, bail, ensure};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(crate) const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 1_000_000;

pub(crate) fn archive_path(name: &str) -> Result<PathBuf> {
	let mut path = PathBuf::new();
	for part in name.split('/') {
		let stem = part.split('.').next().unwrap_or_default().trim_end().to_uppercase();
		let device = stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT"));
		if part.is_empty()
			|| part == "."
			|| part == ".."
			|| part.ends_with(['.', ' '])
			|| part.encode_utf16().count() > 255
			|| part.chars().any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
			|| matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
			|| device.is_some_and(|n| matches!(n, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
		{
			bail!(ErrorKind::InputError(format!("invalid archive path: {name}")));
		}
		path.push(part);
	}
	Ok(path)
}

pub(crate) fn path_is_link(metadata: &fs::Metadata) -> bool {
	#[cfg(windows)]
	{
		use std::os::windows::fs::MetadataExt;
		const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
		if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
			return true;
		}
	}
	metadata.file_type().is_symlink()
}

pub(crate) fn checked_dir(path: &Path) -> Result<()> {
	let metadata = fs::symlink_metadata(path)?;
	ensure!(metadata.is_dir() && !path_is_link(&metadata), "invalid archive directory: {}", path.display());
	Ok(())
}

pub(crate) fn checked_parent_dirs(root: &Path, path: &Path) -> Result<()> {
	let relative = path.strip_prefix(root)?;
	let mut current = root.to_owned();
	for part in relative.components().take(relative.components().count().saturating_sub(1)) {
		current.push(part);
		checked_dir(&current)?;
	}
	Ok(())
}

fn options(size: u64) -> SimpleFileOptions {
	SimpleFileOptions::default()
		.compression_method(CompressionMethod::Deflated)
		.large_file(size >= u64::from(u32::MAX))
}

pub(crate) type ArchiveWriter = ZipWriter<File>;

pub(crate) fn write_archive(
	output: &Path,
	write: impl FnOnce(&mut ArchiveWriter) -> Result<()>,
) -> Result<u64> {
	let parent = output.parent().context("archive output has no parent folder")?;
	let temp = tempfile::NamedTempFile::new_in(parent)?;
	let mut zip = ZipWriter::new(temp.reopen()?);
	write(&mut zip)?;
	let file = zip.finish()?;
	file.sync_all()?;
	let size = file.metadata()?.len();
	drop(file);
	temp.persist_noclobber(output).map_err(|error| error.error)?;
	Ok(size)
}

pub(crate) fn write_bytes(zip: &mut ArchiveWriter, name: &str, bytes: &[u8]) -> Result<()> {
	zip.start_file(name, options(bytes.len() as u64))?;
	zip.write_all(bytes)?;
	Ok(())
}

pub(crate) fn write_tree(zip: &mut ArchiveWriter, root: &Path, prefix: &str) -> Result<()> {
	write_tree_with_files(zip, root, prefix, &HashMap::new())
}

pub(crate) fn write_tree_with_files(
	zip: &mut ArchiveWriter,
	root: &Path,
	prefix: &str,
	managed_files: &HashMap<PathBuf, PathBuf>,
) -> Result<()> {
	checked_dir(root)?;
	archive_path(prefix)?;
	let mut pending = vec![(root.to_path_buf(), prefix.to_owned())];
	while let Some((directory, relative)) = pending.pop() {
		zip.add_directory(format!("{relative}/"), SimpleFileOptions::default())?;
		for child in fs::read_dir(directory)? {
			let child = child?;
			let filename = child.file_name().into_string().map_err(|_| {
				ErrorKind::InputError("archive contains a name that is not UTF-8".into())
			})?;
			if filename == "session.lock" {
				continue;
			}
			let name = format!("{relative}/{filename}");
			archive_path(&name)?;
			let instance_path = child.path();
			let path = managed_files.get(&instance_path).cloned().unwrap_or(instance_path);
			let metadata = fs::symlink_metadata(&path)?;
			ensure!(!path_is_link(&metadata), "archive source contains a linked path: {}", path.display());
			if metadata.is_dir() {
				pending.push((path, name));
			} else {
				ensure!(metadata.is_file(), "unsupported archive source: {}", path.display());
				let mut file = File::open(path)?;
				zip.start_file(name, options(metadata.len()))?;
				io::copy(&mut file, zip)?;
			}
		}
	}
	Ok(())
}

pub(crate) fn extract_archive(archive: &Path, destination: &Path) -> Result<()> {
	extract_archive_with_limits(archive, destination, &[])
}

pub(crate) fn extract_archive_with_limits(
	archive: &Path,
	destination: &Path,
	file_limits: &[(&str, u64)],
) -> Result<()> {
	checked_dir(destination)?;
	let mut zip = ZipArchive::new(File::open(archive)?)?;
	ensure!(zip.len() <= MAX_ENTRIES, "archive has too many files");
	let mut total_size = 0u64;
	let mut known = HashMap::<String, (PathBuf, bool, bool)>::new();
	for index in 0..zip.len() {
		let mut entry = zip.by_index(index)?;
		let raw_name = std::str::from_utf8(entry.name_raw()).context("invalid UTF-8 archive path")?.to_owned();
		let is_dir = raw_name.ends_with('/');
		let name = raw_name.strip_suffix('/').unwrap_or(&raw_name);
		let path = archive_path(name)?;
		if let Some(mode) = entry.unix_mode() {
			ensure!(
				matches!((is_dir, mode & 0o170000), (_, 0) | (true, 0o040000) | (false, 0o100000)),
				"archive contains a special file: {name}"
			);
		}
		for relative in path.ancestors().take_while(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>().into_iter().rev() {
			let is_entry = relative == path;
			let directory = !is_entry || is_dir;
			let folded = relative.to_string_lossy().to_uppercase();
			let existing = known.entry(folded).or_insert_with(|| (relative.to_owned(), directory, false));
			ensure!(
				existing.0 == relative && existing.1 == directory && (!is_entry || !existing.2),
				"archive contains conflicting paths: {name}"
			);
			existing.2 |= is_entry;
		}

		let output = destination.join(path);
		if is_dir {
			ensure!(entry.size() == 0, "archive directory contains data: {name}");
			fs::create_dir_all(output)?;
			continue;
		}
		let size = entry.size();
		if let Some((_, limit)) = file_limits.iter().find(|(path, _)| *path == name) {
			ensure!(size <= *limit, "archive file exceeds size limit: {name}");
		}
		total_size = total_size.checked_add(size).context("archive size exceeds limit")?;
		ensure!(total_size <= MAX_ARCHIVE_BYTES, "archive size exceeds limit");
		if let Some(parent) = output.parent() {
			fs::create_dir_all(parent)?;
		}
		let mut file = File::options().write(true).create_new(true).open(output)?;
		let written = io::copy(&mut (&mut entry).take(size + 1), &mut file)?;
		ensure!(written == size, "archive file size does not match its header: {name}");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn archive_round_trip_keeps_file_bytes_and_empty_directories() -> Result<()> {
		let source = tempfile::tempdir()?;
		let destination = tempfile::tempdir()?;
		let archive = destination.path().join("instance.zip");
		let source_files = source.path().join("mods");
		fs::create_dir(&source_files)?;
		fs::create_dir(source.path().join("empty"))?;
		fs::write(source_files.join("sample.jar.disabled"), b"synthetic mod")?;
		write_archive(&archive, |zip| write_tree(zip, source.path(), "instance"))?;

		let staging = tempfile::tempdir()?;
		extract_archive(&archive, staging.path())?;
		assert_eq!(fs::read(staging.path().join("instance/mods/sample.jar.disabled"))?, b"synthetic mod");
		assert!(staging.path().join("instance/empty").is_dir());
		assert!(write_archive(&archive, |_| Ok(())).is_err());
		assert_eq!(fs::read(source_files.join("sample.jar.disabled"))?, b"synthetic mod");
		Ok(())
	}

	#[test]
	fn archive_rejects_windows_aliases_and_conflicting_entries() -> Result<()> {
		for name in ["../other", "saves/world:save", "saves/CON.txt", "saves/world."] {
			assert!(archive_path(name).is_err(), "{name}");
		}
		let folder = tempfile::tempdir()?;
		let archive = folder.path().join("conflict.zip");
		write_archive(&archive, |zip| {
			write_bytes(zip, "World/level.dat", b"first")?;
			write_bytes(zip, "world/level.dat", b"second")
		})?;
		let staging = tempfile::tempdir()?;
		assert!(extract_archive(&archive, staging.path()).is_err());
		Ok(())
	}
}
