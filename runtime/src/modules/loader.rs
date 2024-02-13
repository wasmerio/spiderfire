/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::collections::hash_map::{Entry, HashMap};
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
		let specifier = request.specifier(cx).to_owned(cx);

		// Do a registry look-up before canonicalizing paths, since the
		// canonicalization process is incompatible with built-in modules
		// that don't have an address on disk.
		if let Some(heap) = self.registry.get(&specifier) {
			return Ok(Module::from_local(heap.root(cx)));
		}

		let path = if !specifier.starts_with('/') {
			Path::new(referencing_module.and_then(|d| d.path.as_ref()).unwrap())
				.parent()
				.unwrap()
				.join(&specifier)
		} else {
			Path::new(&specifier).to_path_buf()
		};

		let path = canonicalize_path(&path).or_else(|e| {
			if path.extension() == Some(OsStr::new("js")) {
				return Err(e);
			}

			// Try appending a .js extension
			let Some(file_name) = path.file_name() else {
				return Err(e);
			};
			let Some(parent) = path.parent() else {
				return Err(e);
			};

			let mut file_name = file_name.to_owned();
			file_name.push(".js");

			canonicalize_path(&parent.join(file_name))
		})?;

		let str = String::from(path.to_str().unwrap());
		match self.registry.get(&str) {
			Some(heap) => Ok(Module::from_local(heap.root(cx))),
			None => {
				let script = read_to_string(&path).map_err(|e| {
					Error::new(
						&format!(
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
					let request = ModuleRequest::new(cx, path.to_str().unwrap());
					self.register(cx, module.module_object(), &request);
					Ok(module)
				} else {
					Err(Error::new(&format!("Unable to compile module: {}\0", specifier), None))
				}
			}
		}
	}

	fn register(&mut self, cx: &Context, module: &Object, request: &ModuleRequest) -> bool {
		let specifier = request.specifier(cx).to_owned(cx);
		match self.registry.entry(specifier) {
			Entry::Vacant(v) => {
				v.insert(TracedHeap::from_local(module));
				true
			}
			Entry::Occupied(_) => false,
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

fn canonicalize_path(path: impl AsRef<Path> + Copy) -> ion::Result<PathBuf> {
	crate::wasi_polyfills::canonicalize(path).map_err(|e| {
		if e.kind() == std::io::ErrorKind::NotFound {
			Error::new(
				format!("Module file not found: {}", path.as_ref().to_string_lossy()).as_str(),
				ion::ErrorKind::Normal,
			)
		} else {
			Error::new(
				format!(
					"IO error {} while trying to canonicalize module path: {}",
					e.kind(),
					path.as_ref().to_string_lossy()
				)
				.as_str(),
				ion::ErrorKind::Normal,
			)
		}
	})
}
