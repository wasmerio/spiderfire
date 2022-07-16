/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::fmt::{Display, Formatter};

use mozjs::jsapi::{JS_ClearPendingException, JS_IsExceptionPending, StackFormat};
use mozjs::rust::jsapi_wrapped::JS_GetPendingException;
use mozjs::rust::MutableHandle;

use crate::{Context, Number, Object, Value};
use crate::value::FromValue;

#[derive(Clone, Debug)]
pub struct Exception {
	pub message: String,
	pub filename: String,
	pub lineno: u32,
	pub column: u32,
}

#[derive(Clone, Debug)]
pub struct ErrorReport {
	pub exception: Exception,
	pub stack: Option<String>,
}

impl Exception {
	/// Gets an exception from the runtime.
	///
	/// Returns [None] is no exception is pending.
	pub fn new(cx: &Context) -> Option<Exception> {
		unsafe {
			if JS_IsExceptionPending(cx.cx()) {
				let mut exception = Value::undefined(cx);
				if JS_GetPendingException(cx.cx(), &mut MutableHandle::from_marked_location(&mut **exception)) {
					let exception = Object::from_raw(cx, exception.to_object());
					Exception::clear(cx);

					let message = crate::String::from_value(cx, exception.get(cx, "message").unwrap())
						.unwrap()
						.to_string(cx);
					let filename = crate::String::from_value(cx, exception.get(cx, "fileName").unwrap())
						.unwrap()
						.to_string(cx);
					let lineno = Number::from_value(cx, exception.get(cx, "lineNumber").unwrap()).unwrap().to_i32() as u32;
					let column = Number::from_value(cx, exception.get(cx, "columnNumber").unwrap()).unwrap().to_i32() as u32;

					Some(Exception { message, filename, lineno, column })
				} else {
					None
				}
			} else {
				None
			}
		}
	}

	/// Clears all exceptions within the runtime.
	pub fn clear(cx: &Context) {
		unsafe { JS_ClearPendingException(cx.cx()) };
	}

	/// Formats the exception as an error message.
	pub fn format(&self) -> String {
		if !self.filename.is_empty() && self.lineno != 0 && self.column != 0 {
			format!(
				"Uncaught exception at {}:{}:{} - {}",
				self.filename, self.lineno, self.column, self.message
			)
		} else {
			format!("Uncaught exception - {}", self.message)
		}
	}
}

impl ErrorReport {
	/// Creates a new [ErrorReport] with the given [Exception] and no stack.
	pub fn new(exception: Exception) -> ErrorReport {
		ErrorReport { exception, stack: None }
	}

	/// Creates a new [ErrorReport] with the given [Exception] and the current stack.
	pub fn new_with_stack(cx: &Context, exception: Exception) -> ErrorReport {
		unsafe {
			capture_stack!(in(cx.cx()) let stack);
			let stack = stack.unwrap().as_string(None, StackFormat::SpiderMonkey);
			ErrorReport { exception, stack }
		}
	}

	pub fn stack(&self) -> Option<&String> {
		self.stack.as_ref()
	}

	/// Prints a formatted error message.
	pub fn print(&self) {
		println!("{}", self);
	}
}

impl Display for ErrorReport {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.exception.format())?;
		if let Some(stack) = self.stack() {
			f.write_str(&format!("\n{}", stack))?;
		}
		Ok(())
	}
}
