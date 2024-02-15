/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::cmp::Ordering;

use mozjs::jsapi::{Heap, JSObject};
use url::Url;

use ion::{ClassDefinition, Context, Object, Result, Error};
use ion::class::Reflector;
use ion::function::Opt;
pub use search_params::URLSearchParams;

mod search_params;

#[derive(Default, FromValue)]
pub struct FormatOptions {
	#[ion(default)]
	auth: bool,
	#[ion(default)]
	fragment: bool,
	#[ion(default)]
	search: bool,
}

#[js_class]
pub struct URL {
	reflector: Reflector,
	#[trace(no_trace)]
	pub(crate) url: Url,
	search_params: Box<Heap<*mut JSObject>>,
}

#[js_class]
impl URL {
	#[ion(constructor)]
	pub fn constructor(#[ion(this)] this: &Object, cx: &Context, input: String, Opt(base): Opt<String>) -> Result<URL> {
		let base = base.as_ref().and_then(|base| Url::parse(base).ok());
		let url = Url::options()
			.base_url(base.as_ref())
			.parse(&input)
			.map_err(|error| Error::new(error.to_string(), None))?;

		let search_params = Box::new(URLSearchParams::new(url.query_pairs().into_owned().collect()));
		search_params.url.as_ref().unwrap().set(this.handle().get());
		let search_params = Heap::boxed(URLSearchParams::new_object(cx, search_params));

		Ok(URL {
			reflector: Reflector::default(),
			url,
			search_params,
		})
	}

	#[ion(name = "canParse")]
	pub fn can_parse(input: String, Opt(base): Opt<String>) -> bool {
		let base = base.as_ref().and_then(|base| Url::parse(base).ok());
		Url::options().base_url(base.as_ref()).parse(&input).is_ok()
	}

	pub fn format(&self, Opt(options): Opt<FormatOptions>) -> Result<String> {
		let mut url = self.url.clone();

		let options = options.unwrap_or_default();
		if !options.auth {
			url.set_username("").map_err(|_| Error::new("Invalid Url", None))?;
		}
		if !options.fragment {
			url.set_fragment(None);
		}
		if !options.search {
			url.set_query(None);
		}

		Ok(url.to_string())
	}

	#[ion(name = "toString", alias = ["toJSON"])]
	#[allow(clippy::inherent_to_string)]
	pub fn to_string(&self) -> String {
		self.url.to_string()
	}

	#[ion(get)]
	pub fn get_href(&self) -> String {
		self.url.to_string()
	}

	#[ion(set)]
	pub fn set_href(&mut self, cx: &Context, input: String) -> Result<()> {
		match url::Url::parse(&input) {
			Ok(url) => {
				let search_params = Box::new(URLSearchParams::new(url.query_pairs().into_owned().collect()));
				search_params.url.as_ref().unwrap().set(self.reflector.get());
				self.search_params = Heap::boxed(URLSearchParams::new_object(cx, search_params));
				self.url = url;
				Ok(())
			}
			Err(error) => Err(Error::new(error.to_string(), None)),
		}
	}

	#[ion(get)]
	pub fn get_protocol(&self) -> String {
		format!("{}:", self.url.scheme())
	}

	#[ion(set)]
	pub fn set_protocol(&mut self, protocol: String) -> Result<()> {
		self.url.set_scheme(&protocol).map_err(|_| Error::new("Invalid Protocol", None))
	}

	#[ion(get)]
	pub fn get_host(&self) -> String {
		self.url
			.host_str()
			.map(|host| {
				if let Some(port) = self.url.port() {
					format!("{}:{}", host, port)
				} else {
					String::from(host)
				}
			})
			.unwrap_or_default()
	}

	#[ion(set)]
	pub fn set_host(&mut self, host: String) -> Result<()> {
		let segments: Vec<&str> = host.split(':').collect();
		let (host, port) = match segments.len().cmp(&2) {
			Ordering::Less => Ok((segments[0], None)),
			Ordering::Greater => Err(Error::new("Invalid Host", None)),
			Ordering::Equal => {
				let port = match segments[1].parse::<u16>() {
					Ok(port) => Ok(port),
					Err(error) => Err(Error::new(error.to_string(), None)),
				}?;
				Ok((segments[0], Some(port)))
			}
		}?;

		self.url.set_host(Some(host))?;
		self.url.set_port(port).map_err(|_| Error::new("Invalid Url", None))
	}

	#[ion(get)]
	pub fn get_hostname(&self) -> String {
		self.url.host_str().map(String::from).unwrap_or_default()
	}

	#[ion(set)]
	pub fn set_hostname(&mut self, hostname: String) -> Result<()> {
		self.url.set_host(Some(&hostname)).map_err(|error| Error::new(error.to_string(), None))
	}

	#[ion(get)]
	pub fn get_origin(&self) -> String {
		self.url.origin().ascii_serialization()
	}

	#[ion(get)]
	pub fn get_port(&self) -> String {
		self.url.port_or_known_default().map(|port| port.to_string()).unwrap_or_default()
	}

	#[ion(set)]
	pub fn set_port(&mut self, port: String) -> Result<()> {
		let port = if port.is_empty() { None } else { Some(port.parse()?) };
		self.url.set_port(port).map_err(|_| Error::new("Invalid Port", None))
	}

	#[ion(get)]
	pub fn get_pathname(&self) -> String {
		String::from(self.url.path())
	}

	#[ion(set)]
	pub fn set_pathname(&mut self, path: String) -> Result<()> {
		self.url.set_path(&path);
		Ok(())
	}

	#[ion(get)]
	pub fn get_username(&self) -> String {
		String::from(self.url.username())
	}

	#[ion(set)]
	pub fn set_username(&mut self, username: String) -> Result<()> {
		self.url.set_username(&username).map_err(|_| Error::new("Invalid Url", None))
	}

	#[ion(get)]
	pub fn get_password(&self) -> String {
		self.url.password().map(String::from).unwrap_or_default()
	}

	#[ion(set)]
	pub fn set_password(&mut self, password: String) -> Result<()> {
		self.url.set_password(Some(&password)).map_err(|_| Error::new("Invalid Url", None))
	}

	#[ion(get)]
	pub fn get_search(&self) -> String {
		self.url.query().map(|search| format!("?{}", search)).unwrap_or_default()
	}

	#[ion(set)]
	pub fn set_search(&mut self, cx: &Context, search: Option<String>) {
		let search = if matches!(search, Some(ref s) if s.as_str() == "?") {
			None
		} else {
			search
		};

		self.url.set_query(search.as_deref());
		URLSearchParams::get_mut_private(cx, &cx.root(self.search_params.get()).into())
			.unwrap()
			.set_pairs(self.url.query_pairs().into_owned().collect())
	}

	#[ion(get)]
	pub fn get_hash(&self) -> String {
		self.url.fragment().map(|hash| format!("#{}", hash)).unwrap_or_default()
	}

	#[ion(set)]
	pub fn set_hash(&mut self, hash: String) {
		self.url.set_fragment(Some(&*hash).filter(|hash| !hash.is_empty()));
	}

	#[ion(get)]
	pub fn get_search_params(&self) -> *mut JSObject {
		self.search_params.get()
	}
}

pub fn define(cx: &Context, global: &Object) -> bool {
	URL::init_class(cx, global).0 && URLSearchParams::init_class(cx, global).0
}
