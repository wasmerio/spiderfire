use std::path::{Component, Path, PathBuf};

pub fn canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
	#[cfg(all(target_os = "wasi", target_vendor = "wasmer"))]
	{
		let path = normalize_path(path, false)?;

		// Get the metadata to ensure the final path exists, as this behavior can be depended
		// on by callers
		std::fs::metadata(&path)?;

		Ok(path)
	}

	#[cfg(not(all(target_os = "wasi", target_vendor = "wasmer")))]
	{
		dunce::canonicalize(path)
	}
}

pub fn normalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
	normalize_path(path, true)
}

fn normalize_path(path: impl AsRef<Path>, dont_touch_fs: bool) -> std::io::Result<PathBuf> {
	let ends_with_slash = path.as_ref().to_str().map_or(false, |s| s.ends_with('/'));
	let mut normalized = PathBuf::new();

	let mut components = path.as_ref().components().peekable();

	if matches!(components.peek(), Some(Component::CurDir)) {
		if dont_touch_fs {
			return Err(std::io::Error::other(
				"Cannot normalize ./ path without touching the FS",
			));
		}

		let cur_dir = std::env::current_dir()?;
		normalized.extend(cur_dir.components());
		components.next();
	}

	for component in components {
		match &component {
			Component::ParentDir => {
				if !normalized.pop() {
					normalized.push(component);
				}
			}
			Component::CurDir => (),
			_ => {
				normalized.push(component);
			}
		}
	}
	if ends_with_slash {
		normalized.push("");
	}

	Ok(normalized)
}
