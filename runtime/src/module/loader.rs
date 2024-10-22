/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::{
	borrow::Cow,
	collections::hash_map::{Entry, HashMap},
};
use std::ffi::OsStr;
use std::fs::read_to_string;
use std::path::{Path, PathBuf};

use mozjs::jsapi::JSObject;
use url::Url;

use ion::{Context, Error, Object, TracedHeap};
use ion::module::{Module, ModuleData, ModuleLoader, ModuleRequest};

use crate::cache::locate_in_cache;
use crate::cache::map::save_sourcemap;
use crate::config::Config;

#[derive(Default)]
pub struct Loader {
	registry: HashMap<String, TracedHeap<*mut JSObject>>,
}

impl ModuleLoader for Loader {
	fn resolve<'cx>(
		&mut self, cx: &'cx Context, referencing_module: Option<&ModuleData>, request: &ModuleRequest,
	) -> ion::Result<Module<'cx>> {
		let specifier = request.specifier(cx).to_owned(cx)?;
		let referencing_path = referencing_module.and_then(|d| d.path.as_ref());

		tracing::info!(specifier, referencing_path, "Resolving module");

		// If the request looks like it's for a built-in module, look it up now.
		if specifier.starts_with("__") || specifier.contains(':') {
			if let Some(heap) = self.registry.get(&specifier) {
				tracing::debug!("Built-in module found in registry");
				return Ok(Module::from_local(heap.root(cx)));
			}
		}

		let raw_path = match referencing_path {
			Some(path) if !specifier.starts_with('/') => Path::new(path).parent().unwrap().join(&specifier),
			_ => Path::new(&specifier).to_path_buf(),
		};

		// Perform a look-up using the path of the module. This helps when the module was
		// loaded before but its file is no longer available, such as when resuming a
		// pre-initialized WASM binary.
		if let Some(heap) = try_locate_registered_module_from_path(&raw_path, &self.registry) {
			return Ok(Module::from_local(heap.root(cx)));
		}

		let path = canonicalize_path(&raw_path).or_else(|e| {
			let path_with_ext = match ensure_extension(&raw_path, "js") {
				Some(Cow::Owned(path)) => path,
				_ => return Err(e),
			};

			tracing::debug!(?path_with_ext, "Failed to find module file, trying with .js extension");

			canonicalize_path(&path_with_ext)
		})?;

		let path_string = String::from(path.to_str().unwrap());
		tracing::debug!(path_string, "Loading module from file");

		let mut raw_path_string = String::from(raw_path.to_str().unwrap());
		if !raw_path_string.ends_with(".js") {
			raw_path_string.push_str(".js");
		}
		let normalized_raw_path_string = crate::wasi_polyfills::normalize(raw_path)
			.map(|p| String::from(p.to_str().unwrap()))
			.ok()
			.unwrap_or(raw_path_string);

		match self.registry.get(&path_string) {
			Some(heap) => {
				tracing::debug!("Found module in registry");
				let module_obj = Object::from(heap.root(cx));

				// Register the module under the new specifier as well
				// to keep future lookups happy
				_ = self.register(cx, &module_obj, normalized_raw_path_string);

				Ok(Module::from_local(module_obj.into_local()))
			}
			None => {
				tracing::debug!("Module not found in registry, loading and compiling");

				let script = read_to_string(&path).map_err(|e| {
					Error::new(
						format!(
							"Unable to read module `{}` from `{}` due to {:?}",
							specifier,
							path.display(),
							e
						),
						None,
					)
				})?;
				let is_typescript = Config::global().typescript && path.extension() == Some(OsStr::new("ts"));
				let (script, sourcemap) = is_typescript
					.then(|| locate_in_cache(&path, &script))
					.flatten()
					.map(|(s, sm)| (s, Some(sm)))
					.unwrap_or_else(|| (script, None));
				if let Some(sourcemap) = sourcemap {
					save_sourcemap(&path, sourcemap);
				}

				let module = Module::compile(cx, &specifier, Some(path.as_path()), &script);

				if let Ok(module) = module {
					// Register the module under both the canonical path and the specifier,
					// so that we find in both of these situations:
					//   * when a different import call refers to the same module file with a different specifier
					//   * when re-importing with the same specifier, without needing to touch the file system
					self.register(cx, module.module_object(), path_string)?;
					_ = self.register(cx, module.module_object(), normalized_raw_path_string);
					Ok(module)
				} else {
					Err(Error::new(format!("Unable to compile module: {}\0", specifier), None))
				}
			}
		}
	}

	fn register(&mut self, cx: &Context, module: &Object, specifier: String) -> ion::Result<()> {
		match self.registry.entry(specifier) {
			Entry::Vacant(v) => {
				tracing::trace!(
					specifier = %v.key(),
					module_object =
						%ion::format::format_value(cx, ion::format::Config::default(), &ion::Value::object(cx, module)),
					"Registring module"
				);
				v.insert(TracedHeap::from_local(module));
				Ok(())
			}
			Entry::Occupied(o) => Err(Error::new(
				format!("Internal error: cannot re-register module with specifier {}", o.key()),
				None,
			)),
		}
	}

	fn metadata(&self, cx: &Context, data: Option<&ModuleData>, meta: &mut Object) -> ion::Result<()> {
		if let Some(data) = data {
			if let Some(path) = data.path.as_ref() {
				let path = canonicalize_path(path)?;
				let url = Url::from_file_path(path).unwrap();
				if !meta.set_as(cx, "url", url.as_str()) {
					return Err(Error::none());
				}
			}
		}

		Ok(())
	}
}

fn try_locate_registered_module_from_path(
	path: impl AsRef<Path>, registry: &HashMap<String, TracedHeap<*mut JSObject>>,
) -> Option<&TracedHeap<*mut JSObject>> {
	// Raw paths are always registered with a .js suffix, so add that now
	let path = ensure_extension(path.as_ref(), "js")?;

	let path_string = String::from(path.as_ref().to_str().unwrap());

	tracing::debug!(path_string, "Trying to locate module");

	// First, try the path directly
	if let Some(heap) = registry.get(&path_string) {
		tracing::debug!("Found module in registry with its raw path");
		return Some(heap);
	}

	// Next, try normalizing the path without checking it exists, to handle
	// cases such as './' or '../'
	if let Some(normalized_path_string) =
		crate::wasi_polyfills::normalize(&path).ok().map(|p| String::from(p.to_str().unwrap()))
	{
		tracing::debug!(normalized_path_string, "Trying to locate module with normalized path");

		if let Some(heap) = registry.get(&normalized_path_string) {
			tracing::debug!("Found module in registry with its normalized raw path");
			return Some(heap);
		}
	}

	None
}

fn ensure_extension<'a>(path: &'a Path, extension: &str) -> Option<Cow<'a, Path>> {
	if path.extension() == Some(OsStr::new(extension)) {
		return Some(Cow::Borrowed(path));
	}

	// Try appending a .js extension
	let mut file_name = path.file_name()?.to_owned();
	file_name.push(".");
	file_name.push(extension);

	Some(Cow::Owned(path.parent()?.join(file_name)))
}

fn canonicalize_path(path: impl AsRef<Path> + Copy) -> ion::Result<PathBuf> {
	crate::wasi_polyfills::canonicalize(path).map_err(|e| {
		if e.kind() == std::io::ErrorKind::NotFound {
			Error::new(
				format!("Module file not found: {}", path.as_ref().to_string_lossy()),
				ion::ErrorKind::Normal,
			)
		} else {
			Error::new(
				format!(
					"IO error {} while trying to canonicalize module path: {}",
					e.kind(),
					path.as_ref().to_string_lossy()
				),
				ion::ErrorKind::Normal,
			)
		}
	})
}
