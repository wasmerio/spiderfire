/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use indent::indent_by;

use crate::{Context, Local};
use crate::format::Config;
use crate::functions::Function;

pub fn format_function<'c>(cx: &Context<'c>, cfg: Config, function: &Local<'c, Function>) -> String {
	indent_by((2 * (cfg.indentation + cfg.depth)) as usize, &function.to_string(cx))
}
