//! `Http` over ureq: rustls, no redirects, error statuses as responses.

use std::time::Duration;

use crate::error::Result;
use crate::http::{transport_error, Http, Response, USER_AGENT};

pub struct UreqHttp {
    agent: ureq::Agent,
}

impl UreqHttp {
    pub fn new() -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // 0 hands a 3xx back to the caller, which is the point: the
            // artifact blob URL must be fetched without the Authorization
            // header, and ureq would carry it over.
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            // Discord's edge rejects a default or empty agent with a 403.
            .user_agent(USER_AGENT)
            .build()
            .into();
        UreqHttp { agent }
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl Http for UreqHttp {
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<Response> {
        let result = match (method, body) {
            ("GET", None) => {
                let mut req = self.agent.get(url);
                for (name, value) in headers {
                    req = req.header(*name, *value);
                }
                req.call()
            }
            ("POST", body) => {
                let mut req = self.agent.post(url);
                for (name, value) in headers {
                    req = req.header(*name, *value);
                }
                req.send(body.unwrap_or(&[]))
            }
            _ => return Err(transport_error(format!("unsupported method {method}"))),
        };
        let mut resp = result.map_err(transport_error)?;
        let status = resp.status().as_u16();
        let mut headers = Vec::new();
        for (name, value) in resp.headers() {
            if let Ok(value) = value.to_str() {
                headers.push((name.as_str().to_lowercase(), value.to_string()));
            }
        }
        let body = resp.body_mut().read_to_vec().map_err(transport_error)?;
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    fn complete(request: &str) -> bool {
        let headers_done = request.contains("\r\n\r\n");
        headers_done && (!request.starts_with("POST") || request.ends_with("{}"))
    }

    /// Serve `replies` one per connection and return the base URL.
    fn serve(replies: Vec<&'static str>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let handle = thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().expect("accept");
                // A POST can arrive as headers then body, so read to the end.
                let mut request = String::new();
                while !complete(&request) {
                    let mut buf = [0u8; 4096];
                    let read = stream.read(&mut buf).expect("read");
                    request.push_str(&String::from_utf8_lossy(&buf[..read]));
                }
                seen.push(request);
                stream.write_all(reply.as_bytes()).expect("write");
            }
            seen
        });
        (base, handle)
    }

    #[test]
    fn a_redirect_is_returned_not_followed() {
        let reply = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/blob\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n";
        let (base, server) = serve(vec![reply]);
        let http = UreqHttp::new();
        let headers = [("Authorization", "Bearer t")];
        let resp = http
            .request("GET", &format!("{base}/zip"), &headers, None)
            .expect("response");
        assert_eq!(resp.status, 302);
        assert_eq!(resp.header("location"), Some("http://127.0.0.1:1/blob"));
        let seen = server.join().expect("server");
        assert!(seen[0].to_lowercase().contains("authorization: bearer t"));
        assert!(seen[0].to_lowercase().contains("user-agent: outl-discussions-sync"));
    }

    #[test]
    fn an_error_status_is_a_response_with_its_body() {
        let reply = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 3\r\n\
                     Content-Length: 2\r\nConnection: close\r\n\r\nhi";
        let (base, server) = serve(vec![reply]);
        let http = UreqHttp::new();
        let headers = [("Content-Type", "application/json")];
        let resp = http
            .request("POST", &format!("{base}/hook"), &headers, Some(b"{}"))
            .expect("response");
        assert_eq!(resp.status, 429);
        assert_eq!(resp.header("retry-after"), Some("3"));
        assert_eq!(resp.body, b"hi");
        let seen = server.join().expect("server");
        assert!(seen[0].starts_with("POST /hook"));
        assert!(seen[0].ends_with("{}"));
    }

    #[test]
    fn a_refused_connection_is_an_error() {
        let http = UreqHttp::new();
        assert!(http.request("GET", "http://127.0.0.1:1/", &[], None).is_err());
    }
}
