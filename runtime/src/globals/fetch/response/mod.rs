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

use ion::{ClassDefinition, Context, Error, ErrorKind, Object, Promise, Result, TracedHeap, Heap};
use ion::class::{NativeObject, Reflector};
use ion::typedarray::ArrayBuffer;
pub use options::*;

use crate::globals::fetch::body::FetchBody;
use crate::globals::fetch::header::HeadersKind;
use crate::globals::fetch::Headers;
use crate::globals::form_data::FormData;
use crate::promise::future_to_promise;

use super::FetchBodyInner;

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
	pub async fn from_hyper_response(cx: &Context, mut response: hyper::Response<Body>, url: Url) -> Result<Response> {
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

		// TODO: support hyper's Body directly, useful for streaming
		let body = response.into_body();
		let bytes = hyper::body::to_bytes(body).await?;

		Ok(Response {
			reflector: Reflector::default(),

			headers: Heap::new(Headers::new_object(&cx, Box::new(headers))),
			body: Some(FetchBody {
				body: FetchBodyInner::Bytes(bytes),
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

	pub fn get_headers_object<'s, 'cx: 's>(&'s mut self, cx: &'cx Context) -> &'s Headers {
		let obj = cx.root_object(self.headers.get()).into();
		Headers::get_private(&obj)
	}

	pub fn take_body_bytes(&mut self) -> Result<Bytes> {
		let body = self.body.take();

		match body {
			None => Err(Error::new("Response body has already been used.", None)),
			Some(body) => Ok(body.into_bytes().unwrap_or_default()),
		}
	}

	pub fn take_body_text(&mut self) -> Result<String> {
		let bytes = self.take_body_bytes()?;
		String::from_utf8(bytes.into()).map_err(|e| Error::new(&format!("Invalid UTF-8 sequence: {}", e), None))
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
			body: Some(FetchBody {
				body: FetchBodyInner::None,
				..Default::default()
			}),

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

	pub fn get_body<'cx>(&mut self, cx: &'cx Context) -> Result<*mut JSObject> {
		let bytes = self.take_body_bytes()?;
		let stream = ion::ReadableStream::from_bytes(&cx, bytes.into());
		Ok((*stream).get())
	}

	#[ion(get, name = "bodyUsed")]
	pub fn get_body_used(&self) -> bool {
		self.body.is_none()
	}

	#[ion(name = "arrayBuffer")]
	pub fn array_buffer<'cx>(&mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |_| async move {
				let mut response = Object::from(this.to_local());
				let response = Response::get_mut_private(&mut response);
				let bytes = response.take_body_bytes()?;
				Ok(ArrayBuffer::from(Vec::from(bytes)))
			})
		}
	}

	pub fn text<'cx>(&mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |_| async move {
				let mut response = Object::from(this.to_local());
				let response = Response::get_mut_private(&mut response);
				let result = response.take_body_text();
				result
			})
		}
	}

	pub fn json<'cx>(&mut self, cx: &'cx Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let mut response = Object::from(this.to_local());
				let response = Response::get_mut_private(&mut response);
				let text = response.take_body_text()?;

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
				let mut response = Object::from(this.to_local());
				let response = Response::get_mut_private(&mut response);

				let bytes = response.take_body_bytes()?;

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
}

pub fn network_error() -> Response {
	Response {
		reflector: Reflector::default(),

		headers: Heap::new(mozjs::jsval::NullValue().to_object_or_null()),
		body: Some(FetchBody {
			body: FetchBodyInner::None,
			..Default::default()
		}),

		kind: ResponseKind::Error,
		url: None,
		redirected: false,

		status: None,
		status_text: None,

		range_requested: false,
	}
}
