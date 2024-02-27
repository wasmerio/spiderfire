/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::str::FromStr;

use bytes::Bytes;
use http::HeaderMap;
use http::header::CONTENT_TYPE;
use hyper::Method;
use ion::string::byte::ByteString;
use ion::{TracedHeap, HeapPointer, Heap, Object};
use ion::typedarray::ArrayBuffer;
use mozjs::jsapi::JSObject;
use url::Url;

use ion::{ClassDefinition, Context, Error, ErrorKind, Result, Promise};
use ion::class::{Reflector, NativeObject};
use ion::function::Opt;
pub use options::*;

use crate::globals::abort::AbortSignal;
use crate::globals::fetch::body::FetchBody;
use crate::globals::fetch::header::HeadersKind;
use crate::globals::fetch::Headers;
use crate::promise::future_to_promise;

use super::body::FetchBodyInner;

mod options;

#[derive(FromValue, Clone)]
pub enum RequestInfo<'cx> {
	#[ion(inherit)]
	Request(&'cx Request),
	#[ion(inherit)]
	String(String),
}

#[js_class]
pub struct Request {
	reflector: Reflector,

	#[trace(no_trace)]
	pub(crate) method: Method,
	pub(crate) headers: Heap<*mut JSObject>,
	pub(crate) body: Option<FetchBody>,
	pub(crate) body_used: bool,

	#[trace(no_trace)]
	pub(crate) locations: Vec<Url>,

	pub(crate) referrer: Referrer,
	pub(crate) referrer_policy: ReferrerPolicy,

	pub(crate) mode: RequestMode,
	pub(crate) credentials: RequestCredentials,
	pub(crate) cache: RequestCache,
	pub(crate) redirect: RequestRedirect,

	pub(crate) integrity: String,

	#[allow(dead_code)]
	pub(crate) unsafe_request: bool,
	pub(crate) keepalive: bool,

	pub(crate) client_window: bool,
	pub(crate) signal_object: Heap<*mut JSObject>,
}

impl Request {
	pub fn url(&self) -> &Url {
		self.locations.last().unwrap()
	}

	pub fn method(&self) -> &Method {
		&self.method
	}

	pub fn headers<'cx>(&self, cx: &'cx Context) -> &'cx HeaderMap {
		&self.get_headers_object(cx).headers
	}

	pub fn get_headers_object<'cx>(&self, cx: &'cx Context) -> &'cx Headers {
		Headers::get_private(cx, &self.headers.root(cx).into()).unwrap()
	}

	pub fn body_if_not_used(&self) -> Result<&FetchBody> {
		match &self.body {
			None => Err(ion::Error::new("Body already used", ion::ErrorKind::Normal)),
			Some(body) => Ok(body),
		}
	}

	pub fn take_body(&mut self) -> Result<FetchBody> {
		if matches!(self.body, Some(FetchBody { ref body, .. }) if matches!(body, FetchBodyInner::None)) {
			return Ok(FetchBody {
				body: FetchBodyInner::None,
				source: None,
				kind: None,
			});
		}
		match self.body.take() {
			None => Err(ion::Error::new("Body already used", ion::ErrorKind::Normal)),
			Some(body) => Ok(body),
		}
	}

	async fn take_body_text(this: &impl HeapPointer<*mut JSObject>, cx: Context) -> Result<String> {
		let this = Self::get_mut_private(&cx, &cx.root(this.to_ptr()).into()).unwrap();
		Ok(this
			.take_body()?
			.into_bytes(cx)
			.await?
			.map(|body| String::from_utf8_lossy(body.as_ref()).into_owned())
			.unwrap_or_else(String::new))
	}

	pub fn try_clone(&mut self, cx: &Context) -> Result<Self> {
		let method = self.method.clone();

		let url = self.locations.last().unwrap().clone();

		Ok(Request {
			reflector: Reflector::default(),

			method,
			headers: Heap::new(Headers::new_object(cx, Box::new(self.get_headers_object(cx).clone()))),
			body: self.body.as_mut().map(|b| b.try_clone(cx)).transpose()?,
			body_used: self.body_used,

			locations: vec![url],

			referrer: self.referrer.clone(),
			referrer_policy: self.referrer_policy,

			mode: self.mode,
			credentials: self.credentials,
			cache: self.cache,
			redirect: self.redirect,

			integrity: self.integrity.clone(),

			unsafe_request: true,
			keepalive: self.keepalive,

			client_window: self.client_window,
			signal_object: Heap::new(self.signal_object.get()),
		})
	}

	pub async fn try_clone_with_cached_body(&mut self, cx: Context) -> Result<Self> {
		let method = self.method.clone();

		let url = self.locations.last().unwrap().clone();

		let headers = Heap::new(Headers::new_object(&cx, Box::new(self.get_headers_object(&cx).clone())));

		let body = match &mut self.body {
			None => None,
			Some(body) => Some(body.try_clone_with_cached_body(cx).await?),
		};

		Ok(Request {
			reflector: Reflector::default(),

			method,
			headers,
			body,
			body_used: self.body_used,

			locations: vec![url],

			referrer: self.referrer.clone(),
			referrer_policy: self.referrer_policy,

			mode: self.mode,
			credentials: self.credentials,
			cache: self.cache,
			redirect: self.redirect,

			integrity: self.integrity.clone(),

			unsafe_request: true,
			keepalive: self.keepalive,

			client_window: self.client_window,
			signal_object: Heap::new(self.signal_object.get()),
		})
	}
}

#[js_class]
impl Request {
	#[ion(constructor)]
	pub fn constructor(cx: &Context, info: RequestInfo, Opt(init): Opt<RequestInit>) -> Result<Request> {
		let mut fallback_cors = false;

		let mut request = match info {
			RequestInfo::Request(request) => {
				let request = Request::get_mut_private(cx, &cx.root(request.reflector().get()).into()).unwrap();
				request.try_clone(cx)?
			}
			RequestInfo::String(url) => {
				let url = Url::from_str(&url)?;
				if url.username() != "" || url.password().is_some() {
					return Err(Error::new("Received URL with embedded credentials", ErrorKind::Type));
				}

				fallback_cors = true;

				Request {
					reflector: Reflector::default(),

					method: Method::GET,
					headers: Heap::new(std::ptr::null_mut()),
					body: Some(FetchBody::default()),
					body_used: false,

					locations: vec![url],

					referrer: Referrer::default(),
					referrer_policy: ReferrerPolicy::default(),

					mode: RequestMode::default(),
					credentials: RequestCredentials::default(),
					cache: RequestCache::default(),
					redirect: RequestRedirect::default(),

					integrity: String::new(),

					unsafe_request: false,
					keepalive: false,

					client_window: true,
					signal_object: Heap::new(AbortSignal::new_object(cx, Box::default())),
				}
			}
		};

		let mut headers = None;
		let mut body = None;

		if let Some(init) = init {
			if init.window.is_some() {
				request.client_window = false;
			}

			if request.mode == RequestMode::Navigate {
				request.mode = RequestMode::SameOrigin;
			}

			if let Some(referrer) = init.referrer {
				request.referrer = referrer;
			}
			if let Some(policy) = init.referrer_policy {
				request.referrer_policy = policy;
			}

			let mode = init.mode.or(fallback_cors.then_some(RequestMode::Cors));
			if let Some(mode) = mode {
				if mode == RequestMode::Navigate {
					return Err(Error::new("Received 'navigate' mode", ErrorKind::Type));
				}
				request.mode = mode;
			}

			if let Some(credentials) = init.credentials {
				request.credentials = credentials;
			}
			if let Some(cache) = init.cache {
				request.cache = cache;
			}
			if let Some(redirect) = init.redirect {
				request.redirect = redirect;
			}
			if let Some(integrity) = init.integrity {
				request.integrity = integrity;
			}
			if let Some(keepalive) = init.keepalive {
				request.keepalive = keepalive;
			}

			if let Some(signal_object) = init.signal {
				request.signal_object.set(signal_object);
			}

			if let Some(mut method) = init.method {
				method.make_ascii_uppercase();
				let method = Method::from_str(&method)?;
				if method == Method::CONNECT || method == Method::TRACE {
					return Err(Error::new("Received invalid request method", ErrorKind::Type));
				}
				request.method = method;
			}

			headers = init.headers;
			body = init.body;
		}

		if request.cache == RequestCache::OnlyIfCached && request.mode != RequestMode::SameOrigin {
			return Err(Error::new(
				"Request cache mode 'only-if-cached' can only be used with request mode 'same-origin'",
				ErrorKind::Type,
			));
		}

		if request.mode == RequestMode::NoCors {
			let method = &request.method;
			if method != Method::GET && method != Method::HEAD && method != Method::POST {
				return Err(Error::new("Invalid request method", ErrorKind::Type));
			}
		}

		let kind = if request.mode == RequestMode::NoCors {
			HeadersKind::RequestNoCors
		} else {
			HeadersKind::Request
		};

		let mut headers = if let Some(headers) = headers {
			headers.into_headers(HeaderMap::new(), kind)?
		} else {
			Headers {
				reflector: Reflector::default(),
				headers: HeaderMap::new(),
				kind,
			}
		};

		if let Some(body) = body {
			body.add_content_type_header(&mut headers.headers);
			request.body = Some(body);
		}
		request.headers.set(Headers::new_object(cx, Box::new(headers)));

		Ok(request)
	}

	#[ion(get)]
	pub fn get_method(&self) -> String {
		self.method.to_string()
	}

	#[ion(get)]
	pub fn get_url(&self) -> String {
		self.url().to_string()
	}

	#[ion(get)]
	pub fn get_headers(&self) -> *mut JSObject {
		self.headers.get()
	}

	#[ion(get)]
	pub fn get_destination(&self) -> String {
		String::new()
	}

	#[ion(get)]
	pub fn get_referrer(&self) -> String {
		self.referrer.to_string()
	}

	#[ion(get)]
	pub fn get_referrer_policy(&self) -> String {
		self.referrer.to_string()
	}

	#[ion(get)]
	pub fn get_mode(&self) -> String {
		self.mode.to_string()
	}

	#[ion(get)]
	pub fn get_credentials(&self) -> String {
		self.credentials.to_string()
	}

	#[ion(get)]
	pub fn get_cache(&self) -> String {
		self.cache.to_string()
	}

	#[ion(get)]
	pub fn get_redirect(&self) -> String {
		self.redirect.to_string()
	}

	#[ion(get)]
	pub fn get_integrity(&self) -> String {
		self.integrity.clone()
	}

	#[ion(get)]
	pub fn get_keepalive(&self) -> bool {
		self.keepalive
	}

	#[ion(get)]
	pub fn get_is_reload_navigation(&self) -> bool {
		false
	}

	#[ion(get)]
	pub fn get_is_history_navigation(&self) -> bool {
		false
	}

	#[ion(get)]
	pub fn get_signal(&self) -> *mut JSObject {
		self.signal_object.get()
	}

	#[ion(get)]
	pub fn get_duplex(&self) -> String {
		String::from("half")
	}

	#[ion(get)]
	pub fn get_body(&mut self, cx: &Context) -> ion::Result<*mut JSObject> {
		let body = self.take_body()?;
		let stream = match body.body {
			FetchBodyInner::None => ion::ReadableStream::from_bytes(cx, Bytes::from(vec![])),
			FetchBodyInner::Bytes(bytes) => ion::ReadableStream::from_bytes(cx, bytes),
			FetchBodyInner::Stream(stream) => stream,
		};
		Ok(stream.get())
	}

	#[ion(get, name = "bodyUsed")]
	pub fn get_body_used(&self) -> bool {
		self.body.is_none()
	}

	#[ion(name = "arrayBuffer")]
	pub fn array_buffer<'cx>(&'cx mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector.get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let this = Self::get_mut_private(&cx, &this.root(&cx).into()).unwrap();
				let body = this.take_body()?;
				let (cx, bytes) = cx.await_native_cx(|cx| body.into_bytes(cx)).await;
				let array = match bytes? {
					Some(ref bytes) => ArrayBuffer::copy_from_bytes(&cx, bytes.as_ref())
						.ok_or_else(|| Error::new("Failed to allocate array", ErrorKind::Normal))?,
					None => ArrayBuffer::copy_from_bytes(&cx, b"")
						.ok_or_else(|| Error::new("Failed to allocate array", ErrorKind::Normal))?,
				};
				Ok(array.get())
			})
		}
	}

	pub fn blob(&mut self, cx: &Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector.get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let this = Self::get_mut_private(&cx, &this.root(&cx).into()).unwrap();
				let body = this.take_body()?;
				let headers = this.get_headers_object(&cx);
				let header = headers.get(ByteString::from(CONTENT_TYPE.to_string().into()).unwrap()).unwrap();
				body.into_blob(cx, header).await
			})
		}
	}

	pub fn text<'cx>(&'cx mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector.get());
		unsafe { future_to_promise(cx, move |cx| async move { Self::take_body_text(&this, cx).await }) }
	}

	pub fn json<'cx>(&'cx mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector.get());
		unsafe {
			future_to_promise(cx, move |cx| async move {
				let body = Self::get_mut_private(&cx, &cx.root(this.to_ptr()).into()).unwrap().take_body()?;
				body.into_json(cx).await
			})
		}
	}

	#[ion(name = "formData")]
	pub fn form_data<'cx>(&'cx mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let this = Self::get_mut_private(&cx, &Object::from(this.to_local())).unwrap();
				let headers = this.get_headers_object(&cx);
				let content_type_string = ByteString::from(CONTENT_TYPE.to_string().into_bytes()).unwrap();
				let Some(content_type) = headers.get(content_type_string)? else {
					return Err(Error::new(
						"No content-type header, cannot decide form data format",
						ErrorKind::Type,
					));
				};
				this.take_body()?.into_form_data(cx, content_type).await
			})
		}
	}

	pub fn clone(&mut self, cx: &Context) -> Result<*mut JSObject> {
		let cloned = self.try_clone(cx)?;
		Ok(Request::new_object(cx, Box::new(cloned)))
	}
}
