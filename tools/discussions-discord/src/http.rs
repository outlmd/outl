//! The seam between the logic and the network.
//!
//! Everything that talks to GitHub or Discord goes through [`Http`], so the
//! tests swap in a fake and never touch a socket.
//! The real implementation is `UreqHttp` in `ureq_http.rs`.

use crate::error::{Result, SyncError};

pub const USER_AGENT: &str = "outl-discussions-sync (https://github.com/outlmd/outl)";

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Response {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_lowercase(), value.to_string()));
        self
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }
}

pub trait Http {
    /// One HTTP call.
    ///
    /// Redirects are NOT followed and an error status is a normal `Response`:
    /// only a transport failure is an `Err`.
    /// Not following redirects matters for the artifact download, whose
    /// redirect target must be fetched without our token.
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<Response>;
}

pub fn transport_error(detail: impl std::fmt::Display) -> SyncError {
    SyncError::new(format!("request failed: {detail}"))
}
