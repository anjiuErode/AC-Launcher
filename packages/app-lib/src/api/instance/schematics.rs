use crate::State;
use crate::api::instance::files::resolve;
use crate::state::instances::adapters::sqlite::instance_rows;
use anyhow::bail;
use quartz_nbt::NbtCompound;
use serde::Serialize;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tokio::fs;

const MAX_SCHEMATICS: usize = 2000;
const MAX_DEPTH: usize = 48;
const MAX_TAGS: usize = 250_000;
const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct SchematicFile {
	pub name: String,
	pub path: String,
	pub extension: String,
	pub size: u64,
	pub modified: u64,
}

#[derive(Debug, Serialize)]
pub struct SchematicDetails {
	pub name: Option<String>,
	pub author: Option<String>,
	pub dimensions: Option<String>,
	pub declared_blocks: Option<i64>,
}

pub async fn list_schematics(instance_id: &str) -> crate::Result<Vec<SchematicFile>> {
	let state = State::get().await?;
	let instance = instance_rows::get_instance_by_id(instance_id, &state.pool)
		.await?
		.ok_or_else(|| crate::ErrorKind::InputError("unknown instance".into()))?;
	let root = state.content_store.instance_path(&instance.path, "schematics").await?;
	match fs::symlink_metadata(root.as_path()).await {
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
		Err(error) => return Err(error.into()),
		Ok(_) => {}
	}
	if fs::symlink_metadata(root.as_path()).await?.file_type().is_symlink() {
		bail!("schematics directory cannot be a symbolic link");
	}
	let mut pending = vec![(root.clone(), 0usize)];
	let mut output = Vec::new();
	let mut visited_directories = 0usize;
	while let Some((directory, depth)) = pending.pop() {
		visited_directories += 1;
		if visited_directories > MAX_SCHEMATICS * 4 {
			break;
		}
		if depth > MAX_DEPTH {
			continue;
		}
		let mut entries = fs::read_dir(&directory).await?;
		while let Some(entry) = entries.next_entry().await? {
			let kind = entry.file_type().await?;
			if kind.is_symlink() {
				continue;
			}
			if kind.is_dir() {
				pending.push((entry.path(), depth + 1));
				continue;
			}
			if !kind.is_file() || !is_schematic(entry.path().as_path()) {
				continue;
			}
			let metadata = entry.metadata().await?;
			let entry_path = entry.path();
			let relative = entry_path.strip_prefix(root.as_path()).unwrap_or(entry_path.as_path());
			output.push(SchematicFile {
				name: entry.file_name().to_string_lossy().into_owned(),
				path: relative.to_string_lossy().replace('\\', "/"),
				extension: extension(entry_path.as_path()).unwrap_or_default(),
				size: metadata.len(),
				modified: metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map_or(0, |time| time.as_secs()),
			});
			if output.len() >= MAX_SCHEMATICS {
				return Ok(output);
			}
		}
	}
	output.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
	Ok(output)
}

pub async fn get_schematic_details(instance_id: &str, relative_path: &str) -> crate::Result<SchematicDetails> {
	let state = State::get().await?;
	let relative_path = validate_schematic_path(relative_path)?;
	let relative = format!("schematics/{relative_path}");
	let content = resolve(&state, instance_id, &relative, false).await?;
	let path = content.path().to_path_buf();
	let instance = instance_rows::get_instance_by_id(instance_id, &state.pool)
		.await?
		.ok_or_else(|| crate::ErrorKind::InputError("unknown instance".into()))?;
	let root = state.content_store.instance_path(&instance.path, "schematics").await?;
	reject_symlink_components(&path, root.as_path()).await?;
	let canonical_root = fs::canonicalize(root.as_path()).await?;
	let canonical_path = fs::canonicalize(&path).await?;
	if !canonical_path.starts_with(canonical_root) {
		bail!("schematic path escapes the schematics directory");
	}
	let metadata = fs::symlink_metadata(&path).await?;
	if metadata.file_type().is_symlink() || !metadata.is_file() {
		bail!("schematic must be a regular file");
	}
	if metadata.len() > MAX_FILE_BYTES {
		bail!("schematic exceeds the 32 MiB limit");
	}
	tokio::task::spawn_blocking(move || parse_schematic(&path))
		.await?
}

pub async fn delete_schematic(instance_id: &str, relative_path: &str) -> crate::Result<()> {
	let path = validate_schematic_path(relative_path)?;
	let state = State::get().await?;
	let instance = instance_rows::get_instance_by_id(instance_id, &state.pool)
		.await?
		.ok_or_else(|| crate::ErrorKind::InputError("unknown instance".into()))?;
	let root = state.content_store.instance_path(&instance.path, "schematics").await?;
	let canonical_root = fs::canonicalize(root.as_path()).await?;
	let _instance = state.instance_locks.lock_content(instance_id).await;
	let _files = state.content_store.files_lock.lock().await;
	let relative = format!("schematics/{path}");
	let content = resolve(&state, instance_id, &relative, true).await?;
	reject_symlink_components(content.path(), root.as_path()).await?;
	let canonical_path = fs::canonicalize(content.path()).await?;
	if !canonical_path.starts_with(canonical_root) || !is_schematic(&canonical_path) {
		bail!("schematic path escapes the schematics directory");
	}
	fs::remove_file(canonical_path).await?;
	Ok(())
}

fn validate_schematic_path(path: &str) -> crate::Result<String> {
	let path = path.replace('\\', "/");
	if path.is_empty() || Path::new(&path).is_absolute() || !is_schematic(Path::new(&path))
		|| path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
	{
		bail!("invalid schematic path");
	}
	Ok(path)
}

async fn reject_symlink_components(path: &Path, root: &Path) -> crate::Result<()> {
	let mut current = path;
	loop {
		if fs::symlink_metadata(current).await?.file_type().is_symlink() {
			bail!("schematic paths cannot contain symbolic links");
		}
		if current == root {
			return Ok(());
		}
		current = current.parent().ok_or_else(|| anyhow::anyhow!("invalid schematic path"))?;
		if !current.starts_with(root) {
			bail!("schematic path escapes the schematics directory");
		}
	}
}

fn extension(path: &Path) -> Option<String> {
	path.extension()?.to_str().map(str::to_ascii_lowercase)
}

fn is_schematic(path: &Path) -> bool {
	matches!(extension(path).as_deref(), Some("litematic" | "schematic" | "schem")) // typos:ignore
}

fn parse_schematic(path: &PathBuf) -> crate::Result<SchematicDetails> {
	let bytes = std::fs::read(path)?;
	if bytes.len() as u64 > MAX_FILE_BYTES {
		bail!("schematic exceeds the 32 MiB limit");
	}
	let data = decompress_bounded(&bytes)?;
	validate_nbt(&data)?;
	let (root, _) = quartz_nbt::io::read_nbt(&mut Cursor::new(data), quartz_nbt::io::Flavor::Uncompressed)?;
	let metadata = root.get::<_, &NbtCompound>("Metadata").ok();
	let name = metadata.and_then(|tag| tag.get::<_, &str>("Name").ok().map(str::to_string));
	let author = metadata.and_then(|tag| tag.get::<_, &str>("Author").ok().map(str::to_string));
	let size = metadata.and_then(|tag| tag.get::<_, &NbtCompound>("EnclosingSize").ok())
		.and_then(dimensions_from_compound)
		.or_else(|| dimensions_from_root(&root));
	let declared_blocks = metadata.and_then(|tag| {
		tag.get::<_, i64>("TotalBlocks").ok()
			.or_else(|| tag.get::<_, i32>("TotalBlocks").ok().map(i64::from))
	});
	Ok(SchematicDetails { name, author, dimensions: size, declared_blocks })
}

fn dimensions_from_compound(tag: &NbtCompound) -> Option<String> {
	let x = tag.get::<_, i32>("x").ok()?;
	let y = tag.get::<_, i32>("y").ok()?;
	let z = tag.get::<_, i32>("z").ok()?;
	Some(format!("{x} × {y} × {z}"))
}

fn dimensions_from_root(root: &NbtCompound) -> Option<String> {
	let x = root.get::<_, i16>("Width").ok()?;
	let y = root.get::<_, i16>("Height").ok()?;
	let z = root.get::<_, i16>("Length").ok()?;
	Some(format!("{x} × {y} × {z}"))
}

fn decompress_bounded(bytes: &[u8]) -> crate::Result<Vec<u8>> {
	use flate2::read::{GzDecoder, ZlibDecoder};
	let mut output = Vec::new();
	if bytes.starts_with(&[0x1f, 0x8b]) {
		GzDecoder::new(bytes).take((MAX_DECOMPRESSED_BYTES + 1) as u64).read_to_end(&mut output)?;
	} else if bytes.len() >= 2 && bytes[0] == 0x78 {
		ZlibDecoder::new(bytes).take((MAX_DECOMPRESSED_BYTES + 1) as u64).read_to_end(&mut output)?;
	} else {
		output.extend_from_slice(bytes);
	}
	if output.len() > MAX_DECOMPRESSED_BYTES {
		bail!("decompressed schematic exceeds the 64 MiB limit");
	}
	Ok(output)
}

fn validate_nbt(bytes: &[u8]) -> crate::Result<()> {
	let mut cursor = Cursor::new(bytes);
	let mut budget = MAX_TAGS;
	let root = read_u8(&mut cursor)?;
	if root != 10 { bail!("unsupported NBT root type"); }
	skip_string(&mut cursor)?;
	validate_payload(&mut cursor, 10, 0, &mut budget)?;
	Ok(())
}

fn validate_payload<R: Read>(reader: &mut R, tag: u8, depth: usize, budget: &mut usize) -> crate::Result<()> {
	if depth > MAX_DEPTH || *budget == 0 { bail!("NBT complexity limit exceeded"); }
	*budget -= 1;
	match tag {
		1 => skip(reader, 1)?, 2 => skip(reader, 2)?, 3 | 5 => skip(reader, 4)?, 4 | 6 => skip(reader, 8)?,
		7 => { let count = read_count(reader, 1)?; skip(reader, count)?; }
		8 => skip_string(reader)?,
		9 => { let child = read_u8(reader)?; let count = read_count(reader, 1)?; if child > 12 || (child == 0 && count > 0) { bail!("invalid NBT list"); } for _ in 0..count { validate_payload(reader, child, depth + 1, budget)?; } }
		10 => loop { let child = read_u8(reader)?; if child == 0 { break; } if child > 12 { bail!("invalid NBT tag type"); } skip_string(reader)?; validate_payload(reader, child, depth + 1, budget)?; },
		11 => { let count = read_count(reader, 4)?; skip(reader, count.checked_mul(4).ok_or_else(|| anyhow::anyhow!("NBT array too large"))?)?; }
		12 => { let count = read_count(reader, 8)?; skip(reader, count.checked_mul(8).ok_or_else(|| anyhow::anyhow!("NBT array too large"))?)?; }
		_ => bail!("invalid NBT tag type"),
	}
	Ok(())
}

fn read_count<R: Read>(reader: &mut R, bytes_per_item: usize) -> crate::Result<usize> {
	let mut bytes = [0; 4]; reader.read_exact(&mut bytes)?;
	let count = i32::from_be_bytes(bytes);
	if count < 0 || count as usize > MAX_DECOMPRESSED_BYTES / bytes_per_item { bail!("NBT array exceeds size limit"); }
	Ok(count as usize)
}

fn read_u8<R: Read>(reader: &mut R) -> crate::Result<u8> { let mut b = [0]; reader.read_exact(&mut b)?; Ok(b[0]) }
fn skip_string<R: Read>(reader: &mut R) -> crate::Result<()> { let mut b = [0; 2]; reader.read_exact(&mut b)?; skip(reader, u16::from_be_bytes(b) as usize) }
fn skip<R: Read>(reader: &mut R, count: usize) -> crate::Result<()> { let mut remaining = count; let mut chunk = [0; 8192]; while remaining > 0 { let take = remaining.min(chunk.len()); reader.read_exact(&mut chunk[..take])?; remaining -= take; } Ok(()) }

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;

	#[test]
	fn schematic_relative_paths_reject_traversal_and_other_types() {
		assert!(validate_schematic_path("../outside.schem").is_err()); // typos:ignore
		assert!(validate_schematic_path("folder\\..\\outside.schem").is_err()); // typos:ignore
		assert!(validate_schematic_path("resourcepack.zip").is_err());
		assert_eq!(validate_schematic_path("nested/house.litematic").unwrap(), "nested/house.litematic");
	}

	#[test]
	fn nbt_validator_rejects_negative_arrays_and_accepts_empty_compound() {
		assert!(validate_nbt(&[10, 0, 0, 0]).is_ok());
		let negative_array = [10, 0, 0, 7, 0, 0, 0, 0xff, 0xff, 0xff, 0xff];
		assert!(validate_nbt(&negative_array).is_err());
	}

	#[test]
	fn parses_declared_litematica_metadata_without_deriving_blocks() {
		let mut metadata = NbtCompound::new();
		metadata.insert("Name", "Test build".to_string());
		metadata.insert("Author", "Builder".to_string());
		metadata.insert("TotalBlocks", 42i64);
		let mut size = NbtCompound::new();
		size.insert("x", 2i32);
		size.insert("y", 3i32);
		size.insert("z", 4i32);
		metadata.insert("EnclosingSize", size);
		let mut root = NbtCompound::new();
		root.insert("Metadata", metadata);
		let mut bytes = Vec::new();
		quartz_nbt::io::write_nbt(&mut bytes, None, &root, quartz_nbt::io::Flavor::GzCompressed).unwrap();
		let file = tempfile::NamedTempFile::new().unwrap();
		std::fs::write(file.path(), bytes).unwrap();
		let details = parse_schematic(&file.path().to_path_buf()).unwrap();
		assert_eq!(details.name.as_deref(), Some("Test build"));
		assert_eq!(details.author.as_deref(), Some("Builder"));
		assert_eq!(details.dimensions.as_deref(), Some("2 × 3 × 4"));
		assert_eq!(details.declared_blocks, Some(42));
	}

	#[test]
	fn missing_declared_block_count_is_not_inferred_from_size() {
		let mut root = NbtCompound::new();
		root.insert("Width", 2i16);
		root.insert("Height", 3i16);
		root.insert("Length", 4i16);
		let mut bytes = Vec::new();
		quartz_nbt::io::write_nbt(&mut bytes, None, &root, quartz_nbt::io::Flavor::Uncompressed).unwrap();
		let file = tempfile::NamedTempFile::new().unwrap();
		std::fs::write(file.path(), bytes).unwrap();
		let details = parse_schematic(&file.path().to_path_buf()).unwrap();
		assert_eq!(details.dimensions.as_deref(), Some("2 × 3 × 4"));
		assert_eq!(details.declared_blocks, None);
	}

	#[test]
	fn parses_zlib_sponge_schematic_metadata() {
		let mut metadata = NbtCompound::new();
		metadata.insert("Name", "Sponge build".to_string());
		metadata.insert("Author", "Sponge author".to_string());
		let mut root = NbtCompound::new();
		root.insert("Width", 5i16);
		root.insert("Height", 6i16);
		root.insert("Length", 7i16);
		root.insert("Metadata", metadata);
		let mut bytes = Vec::new();
		quartz_nbt::io::write_nbt(&mut bytes, None, &root, quartz_nbt::io::Flavor::ZlibCompressed).unwrap();
		let file = tempfile::NamedTempFile::new().unwrap();
		std::fs::write(file.path(), bytes).unwrap();
		let details = parse_schematic(&file.path().to_path_buf()).unwrap();
		assert_eq!(details.name.as_deref(), Some("Sponge build"));
		assert_eq!(details.author.as_deref(), Some("Sponge author"));
		assert_eq!(details.dimensions.as_deref(), Some("5 × 6 × 7"));
		assert_eq!(details.declared_blocks, None);
	}

	#[test]
	fn decompression_limit_rejects_large_expansion() {
		use flate2::write::GzEncoder;
		use flate2::Compression;
		let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
		encoder.write_all(&vec![0; MAX_DECOMPRESSED_BYTES + 1]).unwrap();
		let compressed = encoder.finish().unwrap();
		assert!(decompress_bounded(&compressed).is_err());
	}
}
