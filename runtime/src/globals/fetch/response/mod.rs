/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use bytes::Bytes;
use http::{HeaderMap, HeaderValue};
use http::header::CONTENT_TYPE;
use hyper::{Body, StatusCode};
use hyper::ext::ReasonPhrase;
use ion::string::byte::ByteString;
use mozjs::jsapi::JSObject;
use url::Url;

use ion::{ClassDefinition, Context, Error, ErrorKind, Object, Promise, Result, TracedHeap, Heap, HeapPointer};
use ion::class::{NativeObject, Reflector};
use ion::typedarray::ArrayBuffer;
pub use options::*;

use crate::globals::fetch::body::FetchBody;
use crate::globals::fetch::header::HeadersKind;
use crate::globals::fetch::Headers;
use crate::globals::form_data::FormData;
use crate::promise::future_to_promise;

use super::body::{hyper_body_to_stream, FetchBodyInner};

mod options;

#[js_class]
pub struct Response {
	reflector: Reflector,

	pub(crate) headers: Heap<*mut JSObject>,
	pub(crate) body: Option<FetchBody>,

	pub(crate) kind: ResponseKind,
	#[ion(no_trace)]
	pub(crate) url: Option<Url>,
	pub(crate) redirected: bool,

	#[ion(no_trace)]
	pub(crate) status: Option<StatusCode>,
	pub(crate) status_text: Option<String>,

	pub(crate) range_requested: bool,
}

impl Response {
	pub fn from_hyper_response(cx: &Context, mut response: hyper::Response<Body>, url: Url) -> Result<Response> {
		let status = response.status();
		let status_text = if let Some(reason) = response.extensions().get::<ReasonPhrase>() {
			Some(String::from_utf8(reason.as_bytes().to_vec()).unwrap())
		} else {
			status.canonical_reason().map(String::from)
		};

		let headers = Headers {
			reflector: Reflector::default(),
			headers: std::mem::take(response.headers_mut()),
			kind: HeadersKind::Immutable,
		};

		let body = response.into_body();

		Ok(Response {
			reflector: Reflector::default(),

			headers: Heap::new(Headers::new_object(&cx, Box::new(headers))),
			body: Some(FetchBody {
				body: FetchBodyInner::Stream(hyper_body_to_stream(cx, body).ok_or_else(|| Error::none())?),
				..Default::default()
			}),

			kind: ResponseKind::default(),
			url: Some(url),
			redirected: false,

			status: Some(status),
			status_text,

			range_requested: false,
		})
	}

	pub fn new_from_bytes(bytes: Bytes, url: Url) -> Response {
		Response {
			reflector: Reflector::default(),

			headers: Heap::new(mozjs::jsval::NullValue().to_object_or_null()),
			body: Some(FetchBody {
				body: FetchBodyInner::Bytes(bytes),
				..Default::default()
			}),

			kind: ResponseKind::Basic,
			url: Some(url),
			redirected: false,

			status: Some(StatusCode::OK),
			status_text: Some(String::from("OK")),

			range_requested: false,
		}
	}

	pub fn headers<'cx>(&self, cx: &'cx Context) -> &'cx HeaderMap {
		&Headers::get_private(&self.headers.root(cx).into()).headers
	}

	pub fn get_headers_object<'s, 'cx: 's>(&'s mut self, cx: &'cx Context) -> &'s Headers {
		let obj = cx.root_object(self.headers.get()).into();
		Headers::get_private(&obj)
	}

	pub fn take_body(&mut self) -> Result<FetchBody> {
		let body = self.body.take();

		match body {
			None => Err(Error::new("Response body has already been used.", None)),
			Some(body) => Ok(body),
		}
	}

	pub async fn take_body_bytes(this: &impl HeapPointer<*mut JSObject>, cx: Context) -> Result<Bytes> {
		let body = Self::get_mut_private(&mut cx.root_object(this.to_ptr()).into()).take_body()?;
		Ok(body.into_bytes(cx).await?.unwrap_or_default())
	}

	pub async fn take_body_text(this: &impl HeapPointer<*mut JSObject>, cx: Context) -> Result<String> {
		let bytes = Self::take_body_bytes(this, cx).await?;
		String::from_utf8(bytes.into()).map_err(|e| Error::new(&format!("Invalid UTF-8 sequence: {}", e), None))
	}

	pub fn try_clone(&mut self, cx: &Context) -> Result<Self> {
		Ok(Self {
			reflector: Default::default(),

			headers: self.headers.clone(),
			body: self.body.as_mut().map(|b| b.try_clone(cx)).transpose()?,

			kind: self.kind.clone(),
			url: self.url.clone(),
			redirected: self.redirected,

			status: self.status.clone(),
			status_text: self.status_text.clone(),

			range_requested: self.range_requested,
		})
	}

	pub async fn try_clone_with_cached_body(&mut self, cx: Context) -> Result<Self> {
		let body = match &mut self.body {
			None => None,
			Some(body) => Some(body.try_clone_with_cached_body(cx).await?),
		};

		Ok(Self {
			reflector: Default::default(),

			headers: self.headers.clone(),
			body,

			kind: self.kind.clone(),
			url: self.url.clone(),
			redirected: self.redirected,

			status: self.status.clone(),
			status_text: self.status_text.clone(),

			range_requested: self.range_requested,
		})
	}

	pub fn clone_with_body(&self, body: Option<FetchBody>) -> Self {
		Self {
			reflector: Default::default(),

			headers: self.headers.clone(),
			body,

			kind: self.kind.clone(),
			url: self.url.clone(),
			redirected: self.redirected,

			status: self.status.clone(),
			status_text: self.status_text.clone(),

			range_requested: self.range_requested,
		}
	}
}

#[js_class]
impl Response {
	#[ion(constructor)]
	pub fn constructor(cx: &Context, body: Option<FetchBody>, init: Option<ResponseInit>) -> Result<Response> {
		let init = init.unwrap_or_default();

		let mut response = Response {
			reflector: Reflector::default(),

			headers: Heap::new(mozjs::jsval::NullValue().to_object_or_null()),
			body: Some(FetchBody::default()),

			kind: ResponseKind::default(),
			url: None,
			redirected: false,

			status: Some(init.status),
			status_text: init.status_text,

			range_requested: false,
		};

		let mut headers = init.headers.into_headers(HeaderMap::new(), HeadersKind::Response)?;

		if let Some(body) = body {
			if init.status == StatusCode::NO_CONTENT
				|| init.status == StatusCode::RESET_CONTENT
				|| init.status == StatusCode::NOT_MODIFIED
			{
				return Err(Error::new(
					"Received non-null body with null body status.",
					ErrorKind::Type,
				));
			}

			if let Some(kind) = &body.kind {
				if !headers.headers.contains_key(CONTENT_TYPE) {
					headers.headers.append(CONTENT_TYPE, HeaderValue::from_str(&kind.to_string()).unwrap());
				}
			}
			response.body = Some(body);
		}

		response.headers.set(Headers::new_object(cx, Box::new(headers)));

		Ok(response)
	}

	#[ion(get)]
	pub fn get_type(&self) -> String {
		self.kind.to_string()
	}

	#[ion(get)]
	pub fn get_url(&self) -> String {
		self.url.as_ref().map(Url::to_string).unwrap_or_default()
	}

	#[ion(get)]
	pub fn get_redirected(&self) -> bool {
		self.redirected
	}

	#[ion(get)]
	pub fn get_status(&self) -> u16 {
		self.status.as_ref().map(StatusCode::as_u16).unwrap_or_default()
	}

	#[ion(get)]
	pub fn get_ok(&self) -> bool {
		self.status.as_ref().map(StatusCode::is_success).unwrap_or_default()
	}

	#[ion(get)]
	pub fn get_status_text(&self) -> String {
		self.status_text.clone().unwrap_or_default()
	}

	#[ion(get)]
	pub fn get_headers(&self) -> *mut JSObject {
		self.headers.get()
	}

	#[ion(get)]
	pub fn get_body<'cx>(&mut self, cx: &Context) -> Result<*mut JSObject> {
		let stream = match self.take_body()?.body {
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
	pub fn array_buffer<'cx>(&mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let bytes = Self::take_body_bytes(&this, cx).await?;
				Ok(ArrayBuffer::from(Vec::from(bytes)))
			})
		}
	}

	pub fn text<'cx>(&mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move { Self::take_body_text(&this, cx).await })
		}
	}

	pub fn json<'cx>(&mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let (cx, text) = cx.await_native_cx(|cx| Self::take_body_text(&this, cx)).await;
				let text = text?;

				let Some(str) = ion::String::copy_from_str(&cx, text.as_str()) else {
					return Err(ion::Error::new("Failed to allocate string", ion::ErrorKind::Normal));
				};
				let mut result = ion::Value::undefined(&cx);
				if !mozjs::jsapi::JS_ParseJSON1(cx.as_ptr(), str.handle().into(), result.handle_mut().into()) {
					return Err(ion::Error::new("Failed to deserialize JSON", ion::ErrorKind::Normal));
				}

				Ok((*result.to_object(&cx)).get())
			})
		}
	}

	#[ion(name = "formData")]
	pub fn form_data<'cx>(&mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let (cx, bytes) = cx.await_native_cx(|cx| Self::take_body_bytes(&this, cx)).await;
				let bytes = bytes?;

				let mut response = Object::from(this.to_local());
				let response = Response::get_mut_private(&mut response);

				let headers = response.get_headers_object(&cx);
				let content_type_string = ByteString::<_>::from(CONTENT_TYPE.to_string().into_bytes()).unwrap();
				let Some(content_type) = headers.get(content_type_string)? else {
					return Err(Error::new(
						"No content-type header, cannot decide form data format",
						ErrorKind::Type,
					));
				};
				let content_type = content_type.to_string();

				if content_type.starts_with("application/x-www-form-urlencoded") {
					let parsed = form_urlencoded::parse(bytes.as_ref());
					let mut form_data = FormData::constructor();

					for (key, val) in parsed {
						form_data.append_native_string(key.into_owned(), val.into_owned());
					}

					Ok(FormData::new_object(&cx, Box::new(form_data)))
				} else if content_type.starts_with("multipart/form-data") {
					Err(Error::new(
						"multipart/form-data deserialization is not supported yet",
						ErrorKind::Normal,
					))
				} else {
					Err(Error::new(
						"Invalid content-type, cannot decide form data format",
						ErrorKind::Type,
					))
				}
			})
		}
	}

	pub fn clone(&mut self, cx: &Context) -> Result<*mut JSObject> {
		let cloned = self.try_clone(cx)?;
		Ok(Response::new_object(cx, Box::new(cloned)))
	}
}

pub fn network_error() -> Response {
	Response {
		reflector: Reflector::default(),

		headers: Heap::new(mozjs::jsval::NullValue().to_object_or_null()),
		body: Some(FetchBody::default()),

		kind: ResponseKind::Error,
		url: None,
		redirected: false,

		status: None,
		status_text: None,

		range_requested: false,
	}
}
