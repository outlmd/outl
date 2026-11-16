//! Everything the logic reads from outside, in one value the tests can fake.

use std::collections::HashMap;
use std::time::Duration;

use crate::http::Http;

pub type Env = HashMap<String, String>;

pub struct Ctx<'a> {
    pub env: &'a Env,
    pub http: &'a dyn Http,
    /// Behind a closure so the tests do not really sleep on a 429.
    pub sleep: &'a dyn Fn(Duration),
    /// Where `::notice::` and friends go (stdout, for the Actions runner).
    pub log: &'a dyn Fn(&str),
}

impl<'a> Ctx<'a> {
    pub fn var(&self, name: &str) -> Option<&'a str> {
        self.env.get(name).map(String::as_str)
    }

    pub fn log(&self, line: &str) {
        (self.log)(line);
    }
}
