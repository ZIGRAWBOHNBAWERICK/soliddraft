//! The server behind the web build of SolidDraft.
//!
//! The application and Catallax run in the browser. The server serves their static
//! files and relays assistant requests to the configured provider.
//! API keys stay on the server. `GET /api/info` reports the assistant's state and
//! `GET /version` identifies the running build.

use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use serde_json::json;
use soliddraft_assistant::Provider;
use soliddraft_assistant::relay;
use soliddraft_assistant::wire::ChatRequest;
use tiny_http::{Header, Method, Request, Response, StatusCode};

/// Largest request body accepted, in bytes.
const MAX_BODY: u64 = 64 << 20;

/// Minimum reply size for on-the-fly gzip.
const COMPRESS_FROM: usize = 16 << 10;

/// What the server runs on.
pub struct Config {
    /// The directory holding the web build (`index.html` and the WebAssembly).
    pub web_dir: PathBuf,
    /// The assistant's provider, or why there is none.
    pub provider: Result<Arc<dyn Provider>, String>,
    /// The commit the build was made from, if the build said (`GET /version`).
    pub commit: Option<String>,
}

struct Shared {
    config: Config,
}

/// A running server.
pub struct Server {
    http: Arc<tiny_http::Server>,
    shared: Arc<Shared>,
}

impl Server {
    /// Binds `addr`. Port 0 picks a free port; [`Server::addr`] says which.
    pub fn bind(addr: &str, config: Config) -> Result<Server, String> {
        let http = tiny_http::Server::http(addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
        Ok(Server { http: Arc::new(http), shared: Arc::new(Shared { config }) })
    }

    /// The address the server listens on.
    pub fn addr(&self) -> SocketAddr {
        self.http.server_addr().to_ip().expect("an IP listener")
    }

    /// Serves until the process ends. Each request gets its own thread, since an assistant
    /// reply streams for as long as the model writes.
    pub fn run(self) {
        for request in self.http.incoming_requests() {
            let shared = self.shared.clone();
            let spawned = std::thread::Builder::new().name("request".into()).spawn(move || handle(&shared, request));
            if let Err(e) = spawned {
                log::error!("cannot start a request thread: {e}");
            }
        }
    }

    /// Serves on a background thread, for tests.
    pub fn spawn(self) -> SocketAddr {
        let addr = self.addr();
        std::thread::spawn(move || self.run());
        addr
    }
}

fn handle(shared: &Shared, request: Request) {
    let path = request.url().split('?').next().unwrap_or("/").to_owned();
    let method = request.method().clone();
    let (result, static_asset) = match (&method, path.as_str()) {
        (Method::Get, "/api/info") => (Ok(info(shared)), false),
        (Method::Get, "/version") => (Ok(version(shared)), false),
        (Method::Post, "/api/assistant") => return assistant(shared, request),
        (Method::Get | Method::Head, p) if !p.starts_with("/api/") => {
            (static_file(&shared.config.web_dir, p, request.headers()), true)
        }
        _ => (Err((StatusCode(404), "not found".to_owned())), false),
    };
    let response = match result {
        Ok(reply) => reply.into_response(&request),
        Err((status, message)) => Reply::text(status, message).into_response(&request),
    };
    let sent = if static_asset { respond_static(request, response) } else { request.respond(response) };
    if let Err(e) = sent {
        log::debug!("{method} {path}: the client went away: {e}");
    }
}

/// Retire each static connection so a burst of browser asset requests cannot leave
/// queued connections waiting for idle keep-alive sockets in tiny-http's pool.
fn respond_static(request: Request, response: Response<std::io::Cursor<Vec<u8>>>) -> std::io::Result<()> {
    let status = response.status_code();
    let send_body = request.method() == &Method::Get && status != StatusCode(304);
    let mut headers = Vec::new();
    Response::new(status, response.headers().to_vec(), std::io::empty(), response.data_length(), None)
        .with_chunked_threshold(usize::MAX)
        .raw_print(&mut headers, request.http_version().clone(), &[], true, None)?;
    // The serializer ends the header block with a blank line and strips hop-by-hop
    // headers from its input. Insert Connection before that blank line.
    let headers = headers.strip_suffix(b"\r\n").expect("the HTTP header block ends with a blank line");
    let mut writer = request.into_writer();
    writer.write_all(headers)?;
    writer.write_all(b"Connection: close\r\n\r\n")?;
    if send_body {
        writer.write_all(&response.into_reader().into_inner())?;
    }
    writer.flush()
}

type HandlerResult = Result<Reply, (StatusCode, String)>;

/// A complete reply.
struct Reply {
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
    cache: &'static str,
    /// A validator for a static file, so a browser can ask whether its copy is current.
    etag: Option<String>,
    encoding: Option<&'static str>,
}

impl Reply {
    fn json(body: String) -> Reply {
        Reply {
            status: StatusCode(200),
            content_type: "application/json",
            body: body.into_bytes(),
            cache: "no-store",
            etag: None,
            encoding: None,
        }
    }

    fn text(status: StatusCode, text: String) -> Reply {
        Reply {
            status,
            content_type: "text/plain; charset=utf-8",
            body: text.into_bytes(),
            cache: "no-store",
            etag: None,
            encoding: None,
        }
    }

    fn into_response(self, request: &Request) -> Response<std::io::Cursor<Vec<u8>>> {
        let varies = self.body.len() >= COMPRESS_FROM || self.content_type == "application/wasm";
        let gzip = self.encoding.is_none()
            && self.body.len() >= COMPRESS_FROM
            && encoding_quality(request.headers(), "gzip") > 0.0;
        let encoding = self.encoding.or(gzip.then_some("gzip"));
        let etag = self.etag.map(|tag| match encoding {
            Some(encoding) => format!("\"{tag}-{encoding}\""),
            None => format!("\"{tag}\""),
        });
        let unchanged = etag.as_ref().is_some_and(|etag| {
            request.headers().iter().any(|h| h.field.equiv("If-None-Match") && h.value.as_str() == etag)
        });
        let (status, body) = if unchanged {
            (StatusCode(304), Vec::new())
        } else if gzip {
            (self.status, compress(&self.body))
        } else {
            (self.status, self.body)
        };
        let mut response = Response::from_data(body)
            .with_status_code(status)
            .with_header(header("Content-Type", self.content_type))
            .with_header(header("Cache-Control", self.cache));
        if let Some(encoding) = encoding.filter(|_| !unchanged) {
            response.add_header(header("Content-Encoding", encoding));
        }
        if let Some(etag) = etag {
            response.add_header(header("ETag", &etag));
        }
        if varies {
            response.add_header(header("Vary", "Accept-Encoding"));
        }
        response
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("a valid header")
}

fn encoding_quality(headers: &[Header], encoding: &str) -> f32 {
    let mut wildcard = 0.0;
    for entry in headers
        .iter()
        .filter(|header| header.field.equiv("Accept-Encoding"))
        .flat_map(|header| header.value.as_str().split(','))
    {
        let (name, weight) = entry.trim().split_once(';').unwrap_or((entry.trim(), ""));
        let quality = if weight.is_empty() {
            1.0
        } else {
            weight
                .trim()
                .strip_prefix("q=")
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| (0.0..=1.0).contains(value))
                .unwrap_or(0.0)
        };
        if name.trim().eq_ignore_ascii_case(encoding) {
            return quality;
        }
        if name.trim() == "*" {
            wildcard = quality;
        }
    }
    wildcard
}

fn compress(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).expect("writing to memory");
    encoder.finish().expect("writing to memory")
}

/// Offers the smallest prepared representation when the client equally prefers both.
fn compressed_wasm(path: &Path, headers: &[Header]) -> Option<(PathBuf, Vec<u8>, &'static str)> {
    let mut variants =
        [(encoding_quality(headers, "br"), "br", "wasm.br"), (encoding_quality(headers, "gzip"), "gzip", "wasm.gz")];
    if variants[1].0 > variants[0].0 {
        variants.swap(0, 1);
    }
    variants.into_iter().filter(|(quality, _, _)| *quality > 0.0).find_map(|(_, encoding, extension)| {
        let variant = path.with_extension(extension);
        std::fs::read(&variant).ok().map(|body| (variant, body, encoding))
    })
}

fn info(shared: &Shared) -> Reply {
    let assistant = match &shared.config.provider {
        Ok(p) => json!({ "on": true, "provider": p.name(), "model": p.model() }),
        Err(reason) => json!({ "on": false, "reason": reason }),
    };
    Reply::json(json!({ "assistant": assistant }).to_string())
}

fn version(shared: &Shared) -> Reply {
    Reply::json(json!({ "commit": shared.config.commit, "version": env!("CARGO_PKG_VERSION") }).to_string())
}

fn read_body(request: &mut Request) -> Result<String, (StatusCode, String)> {
    let mut body = String::new();
    request
        .as_reader()
        .take(MAX_BODY + 1)
        .read_to_string(&mut body)
        .map_err(|e| (StatusCode(400), format!("cannot read the request: {e}")))?;
    if body.len() as u64 > MAX_BODY {
        return Err((StatusCode(413), "request too large".into()));
    }
    Ok(body)
}

/// Streams one assistant request's reply as JSON lines.
fn assistant(shared: &Shared, mut request: Request) {
    let provider = match &shared.config.provider {
        Ok(p) => p.clone(),
        Err(reason) => {
            let response = Reply::text(StatusCode(503), reason.clone()).into_response(&request);
            let _ = request.respond(response);
            return;
        }
    };
    let chat: ChatRequest = match read_body(&mut request)
        .and_then(|b| serde_json::from_str(&b).map_err(|e| (StatusCode(400), format!("bad assistant request: {e}"))))
    {
        Ok(c) => c,
        Err((status, message)) => {
            let response = Reply::text(status, message).into_response(&request);
            let _ = request.respond(response);
            return;
        }
    };
    // The provider writes lines into a channel on its own thread; the response reads them
    // out as the body, chunk by chunk, so each event reaches the browser as it happens.
    let (tx, rx) = sync_channel::<Vec<u8>>(256);
    std::thread::Builder::new()
        .name("assistant-relay".into())
        .spawn(move || {
            relay::serve(provider.as_ref(), &chat, &mut |line| send_line(&tx, line));
        })
        .expect("spawn the assistant relay");
    let response = Response::new(
        StatusCode(200),
        vec![header("Content-Type", "application/x-ndjson"), header("Cache-Control", "no-store")],
        LineStream { rx, current: Vec::new(), offset: 0 },
        None,
        None,
    );
    if let Err(e) = request.respond(response) {
        log::debug!("assistant reply: the client went away: {e}");
    }
}

fn send_line(tx: &SyncSender<Vec<u8>>, line: &str) -> bool {
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\n');
    tx.send(bytes).is_ok()
}

/// The lines of a relayed reply as a byte stream. Ends when the relay thread does.
struct LineStream {
    rx: Receiver<Vec<u8>>,
    current: Vec<u8>,
    offset: usize,
}

impl Read for LineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.offset == self.current.len() {
            match self.rx.recv() {
                Ok(next) => {
                    self.current = next;
                    self.offset = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.current.len() - self.offset);
        buf[..n].copy_from_slice(&self.current[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }
}

/// A file of the web build. Paths that leave the directory are not found.
fn static_file(root: &Path, url_path: &str, headers: &[Header]) -> HandlerResult {
    let not_found = || (StatusCode(404), "not found".to_owned());
    let relative = url_path.trim_start_matches('/');
    let relative = if relative.is_empty() { "index.html" } else { relative };
    let relative = Path::new(relative);
    if !relative.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(not_found());
    }
    let path = root.join(relative);
    let content_type = match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "mts" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    };
    // trunk names the app's files by content hash, so those can be cached for good. The
    // page keeps its name from build to build: the browser checks
    // it each time and downloads it again only when the ETag changed.
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let hashed = stem
        .trim_end_matches("_bg")
        .rsplit_once('-')
        .is_some_and(|(_, h)| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    let cache = if hashed { "public, max-age=31536000, immutable" } else { "no-cache" };
    let compressed = (content_type == "application/wasm").then(|| compressed_wasm(&path, headers)).flatten();
    let (served_path, body, encoding) = match compressed {
        Some((variant, body, encoding)) => (variant, body, Some(encoding)),
        None => {
            let body = std::fs::read(&path).map_err(|_| not_found())?;
            (path, body, None)
        }
    };
    let modified = std::fs::metadata(&served_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let etag = Some(format!("{modified:x}-{:x}", body.len()));
    Ok(Reply { status: StatusCode(200), content_type, body, cache, etag, encoding })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_paths_stay_inside_the_web_directory() {
        let dir = std::env::temp_dir().join(format!("soliddraft-server-static-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<p>hi</p>").unwrap();
        assert_eq!(static_file(&dir, "/", &[]).unwrap().body, b"<p>hi</p>");
        assert_eq!(static_file(&dir, "/index.html", &[]).unwrap().content_type, "text/html; charset=utf-8");
        for bad in ["/../etc/passwd", "/a/../../x", "//etc/passwd"] {
            assert_eq!(static_file(&dir, bad, &[]).err().map(|e| e.0), Some(StatusCode(404)), "{bad}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
