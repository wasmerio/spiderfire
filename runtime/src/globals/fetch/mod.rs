/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::iter::once;
use std::str;
use std::str::FromStr;

use async_recursion::async_recursion;
use bytes::Bytes;
use const_format::concatcp;
use data_url::DataUrl;
use futures::future::{Either, select};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use http::header::{
	ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, ACCESS_CONTROL_ALLOW_HEADERS, CACHE_CONTROL, CONTENT_ENCODING,
	CONTENT_LANGUAGE, CONTENT_LENGTH, CONTENT_LOCATION, CONTENT_TYPE, HOST, IF_MATCH, IF_MODIFIED_SINCE, IF_NONE_MATCH,
	IF_RANGE, IF_UNMODIFIED_SINCE, LOCATION, PRAGMA, RANGE, REFERER, REFERRER_POLICY, USER_AGENT,
};
use mozjs::jsapi::JSObject;
use sys_locale::get_locales;
use tokio::fs::read;
use url::Url;

pub use body::{FetchBody, FetchBodyInner, FetchBodyKind, FetchBodyLength, hyper_body_to_stream};
pub use client::{default_client, GLOBAL_CLIENT};
pub use header::{Headers, HeaderEntry, HeadersInit, HeadersObject};
use ion::{ClassDefinition, Context, Error, ErrorKind, Exception, Object, Promise, ResultExc, TracedHeap, Result};
use ion::class::Reflector;
use ion::conversions::ToValue;
use ion::flags::PropertyFlags;
pub use request::{Request, RequestInfo, RequestInit};
pub use response::Response;

use crate::globals::abort::AbortSignal;
use crate::globals::fetch::client::Client;
use crate::globals::fetch::header::{FORBIDDEN_RESPONSE_HEADERS, HeadersKind, remove_all_header_entries};
use crate::globals::fetch::request::{
	Referrer, ReferrerPolicy, RequestCache, RequestCredentials, RequestMode, RequestRedirect,
};
use crate::globals::fetch::response::{network_error, ResponseKind, ResponseTaint};
use crate::promise::future_to_promise;
use crate::VERSION;

mod body;
mod client;
mod header;
mod request;
mod response;

const DEFAULT_USER_AGENT: &str = concatcp!("Spiderfire/", VERSION);

// TODO: replace all of the `network_error()`s with better errors

#[js_fn]
fn fetch(cx: &Context, resource: RequestInfo, init: Option<RequestInit>) -> Option<Promise> {
	let request = match Request::constructor(cx, resource, init) {
		Ok(request) => request,
		Err(error) => {
			return Some(Promise::new_rejected(cx, error.as_value(cx)));
		}
	};

	let signal = Object::from(request.signal_object.to_local());
	let signal = AbortSignal::get_private(&signal);
	if let Some(reason) = signal.get_reason() {
		return Some(Promise::new_rejected(cx, reason));
	}

	let mut headers = Object::from(request.headers.to_local());
	let headers = Headers::get_mut_private(&mut headers);
	if !headers.headers.contains_key(ACCEPT) {
		headers.headers.append(ACCEPT, HeaderValue::from_static("*/*"));
	}

	let mut locales = get_locales().enumerate();
	let mut locale_string = locales.next().map(|(_, s)| s).unwrap_or_else(|| String::from("*"));
	for (index, locale) in locales {
		locale_string.push(',');
		locale_string.push_str(&locale);
		locale_string.push_str(";q=0.");
		locale_string.push_str(&(1000 - index).to_string());
	}
	if !headers.headers.contains_key(ACCEPT_LANGUAGE) {
		headers.headers.append(ACCEPT_LANGUAGE, HeaderValue::from_str(&locale_string).unwrap());
	}

	let request = TracedHeap::new(Request::new_object(cx, Box::new(request)));
	unsafe {
		future_to_promise(cx, move |cx| async move {
			let mut request = Object::from(request.to_local());
			let (cx, res) = cx
				.await_native_cx(|cx| fetch_internal(cx, &mut request, GLOBAL_CLIENT.get().unwrap().clone()))
				.await;
			cx.unroot_persistent_object(request.handle().get());
			res
		})
	}
}

pub async fn fetch_internal<'o>(cx: Context, request: &mut Object<'o>, client: Client) -> ResultExc<*mut JSObject> {
	let request = Request::get_mut_private(request);
	let signal = Object::from(request.signal_object.to_local());
	let signal = AbortSignal::get_private(&signal).signal.clone().poll();
	let request_url = request.url.clone();
	let send = main_fetch(cx.duplicate(), request, client, 0);
	let (cx, response) = cx.await_native(select(send, signal)).await;
	let response = match response {
		Either::Left((response, _)) => response.map_err(Exception::Error),
		Either::Right((exception, _)) => Err(Exception::Other(exception)),
	}?;

	if response.kind == ResponseKind::Error {
		Err(Exception::Error(Error::new(
			&format!("Network Error: Failed to fetch from {}", request_url),
			ErrorKind::Type,
		)))
	} else {
		Ok(Response::new_object(&cx, Box::new(response)))
	}
}

static BAD_PORTS: &[u16] = &[
	1,     // tcpmux
	7,     // echo
	9,     // discard
	11,    // systat
	13,    // daytime
	15,    // netstat
	17,    // qotd
	19,    // chargen
	20,    // ftp-data
	21,    // ftp
	22,    // ssh
	23,    // telnet
	25,    // smtp
	37,    // time
	42,    // name
	43,    // nicname
	53,    // domain
	69,    // tftp
	77,    // —
	79,    // finger
	87,    // —
	95,    // supdup
	101,   // hostname
	102,   // iso-tsap
	103,   // gppitnp
	104,   // acr-nema
	109,   // pop2
	110,   // pop3
	111,   // sunrpc
	113,   // auth
	115,   // sftp
	117,   // uucp-path
	119,   // nntp
	123,   // ntp
	135,   // epmap
	137,   // netbios-ns
	139,   // netbios-ssn
	143,   // imap
	161,   // snmp
	179,   // bgp
	389,   // ldap
	427,   // svrloc
	465,   // submissions
	512,   // exec
	513,   // login
	514,   // shell
	515,   // printer
	526,   // tempo
	530,   // courier
	531,   // chat
	532,   // netnews
	540,   // uucp
	548,   // afp
	554,   // rtsp
	556,   // remotefs
	563,   // nntps
	587,   // submission
	601,   // syslog-conn
	636,   // ldaps
	989,   // ftps-data
	990,   // ftps
	993,   // imaps
	995,   // pop3s
	1719,  // h323gatestat
	1720,  // h323hostcall
	1723,  // pptp
	2049,  // nfs
	3659,  // apple-sasl
	4045,  // npp
	5060,  // sip
	5061,  // sips
	6000,  // x11
	6566,  // sane-port
	6665,  // ircu
	6666,  // ircu
	6667,  // ircu
	6668,  // ircu
	6669,  // ircu
	6697,  // ircs-u
	10080, // amanda
];

static SCHEMES: [&str; 4] = ["about", "blob", "data", "file"];

#[async_recursion(?Send)]
async fn main_fetch(cx: Context, request: &mut Request, client: Client, redirections: u8) -> Result<Response> {
	let scheme = request.url.scheme();

	// TODO: Upgrade HTTP Schemes if the host is a domain and matches the Known HSTS Domain List

	let mut taint = ResponseTaint::default();
	let mut opaque_redirect = false;
	let (cx, response) = {
		if request.mode == RequestMode::SameOrigin {
			let response = network_error(&cx);
			(cx, Ok(response))
		} else if SCHEMES.contains(&scheme) {
			cx.await_native_cx(|cx| scheme_fetch(cx, scheme, request.url.clone())).await
		} else if scheme == "https" || scheme == "http" {
			if let Some(port) = request.url.port() {
				if BAD_PORTS.contains(&port) {
					return Ok(network_error(&cx));
				}
			}
			if request.mode == RequestMode::NoCors {
				if request.redirect != RequestRedirect::Follow {
					return Ok(network_error(&cx));
				}
			} else {
				taint = ResponseTaint::Cors;
			}
			let (cx, (response, opaque)) =
				cx.await_native_cx(|cx| http_fetch(cx, request, client, taint, redirections)).await;
			opaque_redirect = opaque;
			(cx, response)
		} else {
			let response = network_error(&cx);
			(cx, Ok(response))
		}
	};
	let mut response = response?;

	let redirected = redirections > 0;
	if redirected || response.kind == ResponseKind::Error {
		response.redirected = redirected;
		return Ok(response);
	}

	response.url.get_or_insert(request.url.clone());

	let mut headers = Object::from(response.headers.to_local());
	let headers = Headers::get_mut_private(&mut headers);

	if !opaque_redirect
		&& taint == ResponseTaint::Opaque
		&& response.status == Some(StatusCode::PARTIAL_CONTENT)
		&& response.range_requested
		&& !headers.headers.contains_key(RANGE)
	{
		let url = response.url.take().unwrap();
		response = network_error(&cx);
		response.url = Some(url);

		// TODO: do we need to keep constructing this network_error response further?
		return Ok(response);
	}

	if !opaque_redirect
		&& (request.method == Method::HEAD
		|| request.method == Method::CONNECT
		|| response.status == Some(StatusCode::SWITCHING_PROTOCOLS)
		|| response.status.as_ref().map(StatusCode::as_u16) == Some(103) // Early Hints
		|| response.status == Some(StatusCode::NO_CONTENT)
		|| response.status == Some(StatusCode::RESET_CONTENT)
		|| response.status == Some(StatusCode::NOT_MODIFIED))
	{
		response.body = Some(FetchBody::default());
	}

	if opaque_redirect {
		response.kind = ResponseKind::OpaqueRedirect;
		response.url = None;
		response.status = None;
		response.status_text = None;
		response.body = Some(FetchBody::default());

		headers.headers.clear();
	} else {
		match taint {
			ResponseTaint::Basic => {
				response.kind = ResponseKind::Basic;

				for name in &FORBIDDEN_RESPONSE_HEADERS {
					remove_all_header_entries(&mut headers.headers, name);
				}
			}
			ResponseTaint::Cors => {
				response.kind = ResponseKind::Cors;

				let mut allows_all = false;
				let allowed: Vec<_> = headers
					.headers
					.get_all(ACCESS_CONTROL_ALLOW_HEADERS)
					.into_iter()
					.map(|v| {
						if v == "*" {
							allows_all = true
						}
						v.clone()
					})
					.collect();
				let mut to_remove = Vec::new();
				if request.credentials != RequestCredentials::Include && allows_all {
					for name in headers.headers.keys() {
						if headers.headers.get_all(name).into_iter().size_hint().1.is_none() {
							to_remove.push(name.clone());
						}
					}
				} else {
					for name in headers.headers.keys() {
						let allowed = allowed.iter().any(|allowed| allowed.as_bytes() == name.as_str().as_bytes());
						if allowed {
							to_remove.push(name.clone());
						}
					}
				}
				for name in to_remove {
					remove_all_header_entries(&mut headers.headers, &name);
				}
				for name in &FORBIDDEN_RESPONSE_HEADERS {
					remove_all_header_entries(&mut headers.headers, name);
				}
			}
			ResponseTaint::Opaque => {
				response.kind = ResponseKind::Opaque;
				response.url = None;
				response.status = None;
				response.status_text = None;
				response.body = Some(FetchBody::default());

				headers.headers.clear();
			}
		}
	}

	Ok(response)
}

async fn scheme_fetch(cx: Context, scheme: &str, url: Url) -> Result<Response> {
	match scheme {
		"about" if url.path() == "blank" => {
			let response = Response::new_from_bytes(&cx, Bytes::default(), url);
			let headers = Headers {
				reflector: Reflector::default(),
				headers: HeaderMap::from_iter(once((
					CONTENT_TYPE,
					HeaderValue::from_static("text/html;charset=UTF-8"),
				))),
				kind: HeadersKind::Immutable,
			};
			response.headers.set(Headers::new_object(&cx, Box::new(headers)));
			Ok(response)
		}
		// TODO: blob: URLs
		"data" => {
			let data_url = match DataUrl::process(url.as_str()) {
				Ok(data_url) => data_url,
				Err(_) => return Ok(network_error(&cx)),
			};

			let (body, _) = match data_url.decode_to_vec() {
				Ok(decoded) => decoded,
				Err(_) => return Ok(network_error(&cx)),
			};
			let mime = data_url.mime_type();
			let mime = format!("{}/{}", mime.type_, mime.subtype);

			let response = Response::new_from_bytes(&cx, Bytes::from(body), url);
			let headers = Headers {
				reflector: Reflector::default(),
				headers: HeaderMap::from_iter(once((CONTENT_TYPE, HeaderValue::from_str(&mime).unwrap()))),
				kind: HeadersKind::Immutable,
			};
			response.headers.set(Headers::new_object(&cx, Box::new(headers)));
			Ok(response)
		}
		"file" => {
			let path = url.to_file_path().unwrap();
			let (cx, read) = cx.await_native(read(path)).await;
			match read {
				Ok(bytes) => {
					let response = Response::new_from_bytes(&cx, Bytes::from(bytes), url);
					let headers = Headers::new(HeadersKind::Immutable);
					response.headers.set(Headers::new_object(&cx, Box::new(headers)));
					Ok(response)
				}
				Err(_) => Ok(network_error(&cx)),
			}
		}
		_ => Ok(network_error(&cx)),
	}
}

async fn http_fetch(
	cx: Context, request: &mut Request, client: Client, taint: ResponseTaint, redirections: u8,
) -> (Result<Response>, bool) {
	let (cx, response) = cx.await_native_cx(|cx| http_network_fetch(cx, request, client.clone(), false)).await;
	let Ok(response) = response else {
		return (response, false);
	};
	match response.status {
		Some(status) if status.is_redirection() => match request.redirect {
			RequestRedirect::Follow => (
				http_redirect_fetch(cx, request, response, client, taint, redirections).await,
				false,
			),
			RequestRedirect::Error => (Ok(network_error(&cx)), false),
			RequestRedirect::Manual => (Ok(response), true),
		},
		_ => (Ok(response), false),
	}
}

#[async_recursion(?Send)]
async fn http_network_fetch(cx: Context, req: &mut Request, client: Client, is_new: bool) -> Result<Response> {
	let mut request = req.try_clone(&cx)?;
	let mut headers = Object::from(req.headers.to_local());
	let headers = Headers::get_mut_private(&mut headers);

	let mut request_builder = hyper::Request::builder().method(&request.method).uri(request.url.to_string());
	let Some(request_headers) = request_builder.headers_mut() else {
		return Ok(network_error(&cx));
	};
	*request_headers = headers.headers.clone();
	let headers = request_headers;

	let request_body = match &request.body {
		Some(body) => body,
		None => return Err(Error::new("Request body was already used", ErrorKind::Type)),
	};

	let length = match request_body.len() {
		FetchBodyLength::None => {
			if request.body.is_none() && (request.method == Method::POST || request.method == Method::PUT) {
				Some(0)
			} else {
				None
			}
		}
		FetchBodyLength::Known(l) => Some(l),
		FetchBodyLength::Unknown => None,
	};

	if let Some(length) = length {
		headers.append(CONTENT_LENGTH, HeaderValue::from_str(&length.to_string()).unwrap());
	}

	if let Referrer::Url(url) = request.referrer {
		headers.append(REFERER, HeaderValue::from_str(url.as_str()).unwrap());
	}

	if !headers.contains_key(USER_AGENT) {
		headers.append(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
	}

	if request.cache == RequestCache::Default
		&& (headers.contains_key(IF_MODIFIED_SINCE)
			|| headers.contains_key(IF_NONE_MATCH)
			|| headers.contains_key(IF_UNMODIFIED_SINCE)
			|| headers.contains_key(IF_MATCH)
			|| headers.contains_key(IF_RANGE))
	{
		request.cache = RequestCache::NoStore;
	}

	if request.cache == RequestCache::NoCache && !headers.contains_key(CACHE_CONTROL) {
		headers.append(CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
	}

	if request.cache == RequestCache::NoStore || request.cache == RequestCache::Reload {
		if !headers.contains_key(PRAGMA) {
			headers.append(PRAGMA, HeaderValue::from_static("no-cache"));
		}
		if !headers.contains_key(CACHE_CONTROL) {
			headers.append(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
		}
	}

	if headers.contains_key(RANGE) {
		headers.append(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
	}

	if !headers.contains_key(HOST) {
		let host = request
			.url
			.host_str()
			.map(|host| {
				if let Some(port) = request.url.port() {
					format!("{}:{}", host, port)
				} else {
					String::from(host)
				}
			})
			.unwrap();
		headers.append(HOST, HeaderValue::from_str(&host).unwrap());
	}

	if request.cache == RequestCache::OnlyIfCached {
		return Ok(network_error(&cx));
	}

	let range_requested = headers.contains_key(RANGE);

	// We check for the existence of a request body above, so we can safely unwrap here
	let hyper_body = request.body.unwrap().into_http_body(cx.duplicate());
	let (hyper_body, body_fut) = hyper_body?;
	let Ok(hyper_request) = request_builder.body(hyper_body) else {
		return Ok(network_error(&cx));
	};

	let (cx, hyper_response) = match body_fut {
		None => cx.await_native(client.request(hyper_request)).await,
		Some(f) => {
			// See comment on [FetchBody::into_http_body]. We have to run
			// both futures simultaneously, giving us this lovely bit of
			// code.
			let cx2 = cx.duplicate();
			drop(cx);
			let res = futures::join!(client.request(hyper_request), f);
			(cx2, res.0)
		}
	};
	let hyper_response = hyper_response?;
	let mut response = Response::from_hyper_response(&cx, hyper_response, req.url.clone())?;

	response.range_requested = range_requested;

	if response.status == Some(StatusCode::PROXY_AUTHENTICATION_REQUIRED) && !req.client_window {
		return Ok(network_error(&cx));
	}

	if response.status == Some(StatusCode::MISDIRECTED_REQUEST) && !is_new {
		return http_network_fetch(cx, req, client, true).await;
	}

	Ok(response)
}

async fn http_redirect_fetch(
	cx: Context, request: &mut Request, response: Response, client: Client, taint: ResponseTaint, redirections: u8,
) -> Result<Response> {
	let headers = Object::from(response.headers.to_local());
	let headers = Headers::get_private(&headers);
	let mut location = headers.headers.get_all(LOCATION).into_iter();
	let location = match location.size_hint().1 {
		Some(0) => return Ok(response),
		None => return Ok(network_error(&cx)),
		_ => {
			let location = location.next().unwrap();
			match Url::options()
				.base_url(response.url.as_ref())
				.parse(str::from_utf8(location.as_bytes()).unwrap())
			{
				Ok(mut url) => {
					if url.fragment().is_none() {
						url.set_fragment(response.url.as_ref().and_then(Url::fragment));
					}
					url
				}
				Err(_) => return Ok(network_error(&cx)),
			}
		}
	};

	if !(location.scheme() == "https" || location.scheme() == "http") {
		return Ok(network_error(&cx));
	}

	if redirections >= 20 {
		return Ok(network_error(&cx));
	}

	if taint == ResponseTaint::Cors && (location.username() != "" || location.password().is_some()) {
		return Ok(network_error(&cx));
	}

	if ((response.status == Some(StatusCode::MOVED_PERMANENTLY) || response.status == Some(StatusCode::FOUND))
		&& request.method == Method::POST)
		|| (response.status == Some(StatusCode::SEE_OTHER)
			&& (request.method != Method::GET || request.method != Method::HEAD))
	{
		request.method = Method::GET;
		request.body = Some(FetchBody::default());
		let mut headers = Object::from(request.headers.to_local());
		let headers = Headers::get_mut_private(&mut headers);
		remove_all_header_entries(&mut headers.headers, &CONTENT_ENCODING);
		remove_all_header_entries(&mut headers.headers, &CONTENT_LANGUAGE);
		remove_all_header_entries(&mut headers.headers, &CONTENT_LOCATION);
		remove_all_header_entries(&mut headers.headers, &CONTENT_TYPE);
	}

	request.locations.push(location.clone());
	request.url = location;

	let policy = headers.headers.get_all(REFERRER_POLICY).into_iter().rev();
	let policy = policy
		.filter(|v| !v.is_empty())
		.find_map(|v| ReferrerPolicy::from_str(str::from_utf8(v.as_bytes()).unwrap()).ok());
	if let Some(policy) = policy {
		request.referrer_policy = policy;
	}

	main_fetch(cx, request, client, redirections + 1).await
}

pub fn define(cx: &Context, global: &mut Object) -> bool {
	let _ = GLOBAL_CLIENT.set(default_client());
	global.define_method(cx, "fetch", fetch, 1, PropertyFlags::CONSTANT_ENUMERATED);
	Headers::init_class(cx, global).0 && Request::init_class(cx, global).0 && Response::init_class(cx, global).0
}
