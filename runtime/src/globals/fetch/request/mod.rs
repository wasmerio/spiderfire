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
use ion::{string::byte::ByteString};
use ion::{TracedHeap, HeapPointer, Heap, Object};
use ion::typedarray::{ArrayBufferWrapper, Uint8ArrayWrapper};
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

#[derive(FromValue, Clone, Debug)]
pub enum RequestInfo<'cx> {
	#[ion(inherit)]
	Request(&'cx Request),
	#[ion(inherit)]
	String(String),
}

#[js_class]
#[derive(Debug)]
pub struct Request {
	reflector: Reflector,

	#[trace(no_trace)]
	pub(crate) method: Method,
	pub(crate) headers: Heap<*mut JSObject>,

	/// To gain a bit of speed, we try to keep native buffers around for as long as possible.
	/// This gives rise to a few scenarios:
	///   * The request body is null/empty. In this case, this will be Some(FetchBodyInner::None).
	///     In this case, the body will always remain unused.
	///   * The request body comes from a native source, so it's a buffer of bytes. Then...
	///     * If the body is requested as a stream, we turn it into a stream
	///     * If text/json/array buffer/etc. is requested, we use the bytes to provide that
	///       data and set this to None.
	///   * The request body is a stream. In this case...
	///     * When a stream is requested, we will simply return the stream.
	///     * If text/json/etc. is requested, we use Request::is_body_used (which itself uses
	///       ReadableStream::is_disturbed) to make sure we don't re-use a stream body that's
	///       already been used, then consume the stream and set this to None.
	pub(crate) body: Option<FetchBody>,

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

	pub fn get_headers_object_mut<'cx>(&self, cx: &'cx Context) -> &'cx mut Headers {
		Headers::get_mut_private(cx, &self.headers.root(cx).into()).unwrap()
	}

	pub fn body_if_not_used(&self, cx: &Context) -> Result<&FetchBody> {
		if self.get_body_used(cx) {
			Err(ion::Error::new("Body already used", ion::ErrorKind::Normal))
		} else {
			Ok(self.body.as_ref().unwrap())
		}
	}

	pub fn take_body(&mut self, cx: &Context) -> Result<FetchBody> {
		if self.get_body_used(cx) {
			return Err(ion::Error::new("Body already used", ion::ErrorKind::Normal));
		}

		if matches!(self.body, Some(FetchBody { body: FetchBodyInner::None, .. })) {
			return Ok(FetchBody {
				body: FetchBodyInner::None,
				source: None,
				kind: None,
			});
		}

		Ok(self.body.take().unwrap())
	}

	pub async fn take_body_text(this: &impl HeapPointer<*mut JSObject>, cx: Context) -> Result<String> {
		let body = Self::get_mut_private(&cx, &cx.root(this.to_ptr()).into()).unwrap().take_body(&cx)?;
		body.into_text(cx).await
	}

	pub fn try_clone(&mut self, cx: &Context) -> Result<Self> {
		let method = self.method.clone();

		let url = self.locations.last().unwrap().clone();

		Ok(Request {
			reflector: Reflector::default(),

			method,
			headers: Heap::new(Headers::new_object(cx, Box::new(self.get_headers_object(cx).clone()))),
			body: self.body.as_mut().map(|b| b.try_clone(cx)).transpose()?,

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

		if let Some(headers) = headers {
			request.headers.set(Headers::new_object(
				cx,
				Box::new(headers.into_headers(HeaderMap::new(), kind)?),
			));
		} else if request.headers.get().is_null() {
			request.headers.set(Headers::new_object(
				cx,
				Box::new(Headers {
					reflector: Reflector::default(),
					headers: HeaderMap::new(),
					kind,
				}),
			));
		};

		if let Some(body) = body {
			body.add_content_type_header(&mut request.get_headers_object_mut(cx).headers);
			request.body = Some(body);
		}

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
		if self.get_body_used(cx) {
			return Err(ion::Error::new("Body already used", ion::ErrorKind::Normal));
		}

		let stream = match self.body.as_ref().unwrap().body {
			FetchBodyInner::None => ion::ReadableStream::from_bytes(cx, Bytes::from(vec![])),
			FetchBodyInner::Bytes(_) => {
				let body = self.body.take().unwrap();
				let FetchBodyInner::Bytes(bytes) = body.body else {
					unreachable!()
				};
				let stream = ion::ReadableStream::from_bytes(cx, bytes);
				let new_body = FetchBody {
					body: FetchBodyInner::Stream(stream.clone()),
					..body
				};
				self.body = Some(new_body);
				stream
			}
			FetchBodyInner::Stream(ref stream) => stream.clone(),
		};

		Ok(stream.get())
	}

	#[ion(get, name = "bodyUsed")]
	pub fn get_body_used(&self, cx: &Context) -> bool {
		match &self.body {
			None => true,
			Some(FetchBody { body: FetchBodyInner::Stream(stream), .. }) => stream.is_disturbed(cx),
			_ => false,
		}
	}

	#[ion(name = "arrayBuffer")]
	pub fn array_buffer<'cx>(&'cx mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector.get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let this = Self::get_mut_private(&cx, &this.root(&cx).into()).unwrap();
				let body = this.take_body(&cx)?;
				let (_, bytes) = cx.await_native_cx(|cx| body.into_bytes(cx)).await;
				let bytes = bytes?.unwrap_or_default();
				Ok(ArrayBufferWrapper::from(bytes.as_ref()))
			})
		}
	}

	pub fn bytes<'cx>(&'cx mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector.get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let this = Self::get_mut_private(&cx, &this.root(&cx).into()).unwrap();
				let body = this.take_body(&cx)?;
				let (_, bytes) = cx.await_native_cx(|cx| body.into_bytes(cx)).await;
				let bytes = bytes?.unwrap_or_default();
				Ok(Uint8ArrayWrapper::from(bytes.as_ref()))
			})
		}
	}

	pub fn blob(&mut self, cx: &Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector.get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let this = Self::get_mut_private(&cx, &this.root(&cx).into()).unwrap();
				let body = this.take_body(&cx)?;
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
				let body = Self::get_mut_private(&cx, &cx.root(this.to_ptr()).into()).unwrap().take_body(&cx)?;
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
				this.take_body(&cx)?.into_form_data(cx, content_type).await
			})
		}
	}

	pub fn clone(&mut self, cx: &Context) -> Result<*mut JSObject> {
		let cloned = self.try_clone(cx)?;
		Ok(Request::new_object(cx, Box::new(cloned)))
	}
}
