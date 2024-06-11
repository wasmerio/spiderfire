/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use bytes::Bytes;
use http::{HeaderValue, StatusCode};
use http::header::{CONTENT_TYPE, LOCATION};
use hyper::{Body, HeaderMap};
use hyper::ext::ReasonPhrase;
use ion::conversions::ToValue;
use ion::string::byte::{ByteString, VisibleAscii};
use mozjs::conversions::ConversionBehavior;
use mozjs::jsapi::JSObject;
use url::Url;

use ion::{ClassDefinition, Context, Error, ErrorKind, Heap, HeapPointer, Object, Promise, Result, ResultExc, TracedHeap};
use ion::class::{NativeObject, Reflector};
use ion::function::Opt;
use ion::typedarray::{ArrayBufferWrapper, Uint8ArrayWrapper};
pub use options::*;

use crate::globals::fetch::body::FetchBody;
use crate::globals::fetch::header::HeadersKind;
use crate::globals::fetch::Headers;
use crate::promise::future_to_promise;

use super::HeadersInit;
use super::body::{hyper_body_to_stream, FetchBodyInner};

mod options;

#[js_class]
pub struct Response {
	reflector: Reflector,

	pub(crate) headers: Heap<*mut JSObject>,
	pub(crate) body: Option<FetchBody>,

	pub(crate) kind: ResponseKind,
	#[trace(no_trace)]
	pub(crate) url: Option<Url>,
	pub(crate) redirected: bool,

	#[trace(no_trace)]
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
			response.status().canonical_reason().map(String::from)
		};

		let headers = Headers {
			reflector: Reflector::default(),
			headers: std::mem::take(response.headers_mut()),
			kind: HeadersKind::Immutable,
		};

		let body = response.into_body();

		Ok(Response {
			reflector: Reflector::default(),

			headers: Heap::new(Headers::new_object(cx, Box::new(headers))),
			body: Some(FetchBody {
				body: FetchBodyInner::Stream(hyper_body_to_stream(cx, body).ok_or_else(Error::none)?),
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

	pub fn new_from_bytes(cx: &Context, bytes: Bytes, url: Url) -> Response {
		Response {
			reflector: Reflector::default(),

			headers: Heap::new(Headers::new_object(cx, Box::new(Headers::new(HeadersKind::Response)))),
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
		&Headers::get_private(cx, &self.headers.root(cx).into()).unwrap().headers
	}

	pub fn get_headers_object<'s, 'cx: 's>(&'s mut self, cx: &'cx Context) -> &'s Headers {
		let obj = cx.root(self.headers.get()).into();
		Headers::get_private(cx, &obj).unwrap()
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
			None => Err(Error::new("Response body has already been used.", None)),
			Some(body) => Ok(body),
		}
	}

	pub async fn take_body_bytes(this: &impl HeapPointer<*mut JSObject>, cx: Context) -> Result<Bytes> {
		let body = Self::get_mut_private(&cx, &cx.root(this.to_ptr()).into()).unwrap().take_body()?;
		Ok(body.into_bytes(cx).await?.unwrap_or_default())
	}

	pub async fn take_body_text(this: &impl HeapPointer<*mut JSObject>, cx: Context) -> Result<String> {
		let body = Self::get_mut_private(&cx, &cx.root(this.to_ptr()).into()).unwrap().take_body()?;
		body.into_text(cx).await
	}

	pub fn try_clone(&mut self, cx: &Context) -> Result<Self> {
		Ok(Self {
			reflector: Default::default(),

			headers: self.headers.clone(),
			body: self.body.as_mut().map(|b| b.try_clone(cx)).transpose()?,

			kind: self.kind,
			url: self.url.clone(),
			redirected: self.redirected,

			status: self.status,
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

			kind: self.kind,
			url: self.url.clone(),
			redirected: self.redirected,

			status: self.status,
			status_text: self.status_text.clone(),

			range_requested: self.range_requested,
		})
	}

	pub fn clone_with_body(&self, body: Option<FetchBody>) -> Self {
		Self {
			reflector: Default::default(),

			headers: self.headers.clone(),
			body,

			kind: self.kind,
			url: self.url.clone(),
			redirected: self.redirected,

			status: self.status,
			status_text: self.status_text.clone(),

			range_requested: self.range_requested,
		}
	}
}

#[js_class]
impl Response {
	#[ion(constructor)]
	pub fn constructor(cx: &Context, Opt(body): Opt<FetchBody>, Opt(init): Opt<ResponseInit>) -> Result<Response> {
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

			body.add_content_type_header(&mut headers.headers);
			response.body = Some(body);
		}

		response.headers.set(Headers::new_object(cx, Box::new(headers)));

		Ok(response)
	}

	pub fn error(cx: &Context) -> *mut JSObject {
		Response::new_object(cx, Box::new(network_error(cx)))
	}

	#[ion(name = "json")]
	pub fn static_json(cx: &Context, data: Object, Opt(options): Opt<ResponseInit>) -> ResultExc<*mut JSObject> {
		let text = ion::json::stringify(cx, data.as_value(cx))?;
		let text_bytes: Vec<_> = text.into();
		let body = FetchBody {
			body: FetchBodyInner::Bytes(text_bytes.into()),
			..Default::default()
		};

		let mut options = options.unwrap_or_default();
		let mut headers = options.headers.into_headers(HeaderMap::default(), HeadersKind::Response)?;
		if !headers.headers.contains_key(CONTENT_TYPE) {
			headers.headers.append(CONTENT_TYPE, HeaderValue::from_static("application/json"));
		}
		options.headers = HeadersInit::Existing(&headers);

		Ok(Response::new_object(
			cx,
			Box::new(Response::constructor(cx, Opt(Some(body)), Opt(Some(options)))?),
		))
	}

	pub fn redirect(
		cx: &Context, location: ByteString<VisibleAscii>,
		#[ion(convert = ConversionBehavior::Clamp, strict)] Opt(status): Opt<u16>,
	) -> Result<*mut JSObject> {
		let status = status.unwrap_or(302);
		if ![301, 302, 303, 307, 308].contains(&status) {
			return Err(Error::new("Invalid status code for redirect response", ErrorKind::Type));
		}

		let mut headers = Headers::new(HeadersKind::Response);
		headers.headers.append(
			LOCATION,
			HeaderValue::from_bytes(location.as_bytes())
				.map_err(|_| Error::new("Invalid Location header value", ErrorKind::Type))?,
		);

		let init = ResponseInit {
			status: StatusCode::from_u16(status).unwrap(),
			headers: HeadersInit::Existing(&headers),
			..Default::default()
		};

		Ok(Response::new_object(
			cx,
			Box::new(Response::constructor(cx, Opt(None), Opt(Some(init)))?),
		))
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
	pub fn get_body(&mut self, cx: &Context) -> Result<*mut JSObject> {
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
	pub fn array_buffer(&mut self, cx: &Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
				let bytes = Self::take_body_bytes(&this, cx).await?;
				Ok(ArrayBufferWrapper::from(Vec::from(bytes)))
			})
		}
	}

	// TODO: the inclusion of this method causes problems with requests
	// not finishing (undefined behavior?), commented out for 1.2.0 release

	// pub fn bytes(&mut self, cx: &Context) -> Option<Promise> {
	// 	let this = TracedHeap::new(self.reflector().get());
	// 	unsafe {
	// 		future_to_promise::<_, _, _, Error>(cx, move |cx| async move {
	// 			let bytes = Self::take_body_bytes(&this, cx).await?;
	// 			Ok(Uint8ArrayWrapper::from(Vec::from(bytes)))
	// 		})
	// 	}
	// }

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

	pub fn text(&mut self, cx: &Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, _, Error>(cx, move |cx| async move { Self::take_body_text(&this, cx).await })
		}
	}

	pub fn json(&mut self, cx: &Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector.get());
		unsafe {
			future_to_promise(cx, move |cx| async move {
				let body = Self::get_mut_private(&cx, &cx.root(this.to_ptr()).into()).unwrap().take_body()?;
				body.into_json(cx).await
			})
		}
	}

	#[ion(name = "formData")]
	pub fn form_data(&mut self, cx: &Context) -> Option<Promise> {
		let this = TracedHeap::new(self.reflector().get());
		unsafe {
			future_to_promise::<_, _, *mut JSObject, Error>(cx, move |cx| async move {
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
		Ok(Response::new_object(cx, Box::new(cloned)))
	}
}

pub fn network_error(cx: &Context) -> Response {
	Response {
		reflector: Reflector::default(),

		headers: Heap::new(Headers::new_object(cx, Box::new(Headers::new(HeadersKind::Response)))),
		body: Some(FetchBody::default()),

		kind: ResponseKind::Error,
		url: None,
		redirected: false,

		status: None,
		status_text: None,

		range_requested: false,
	}
}
