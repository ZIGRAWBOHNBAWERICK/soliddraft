//! Native drawing requests run on a part snapshot, with a bounded reply to the client.

use crate::app_tools::AppTools;
use crate::command_tools::{Surface, tool_specs};
use crate::drawing::{DrawingSession, NewDrawing};
use crate::state::AppState;
use serde_json::Value;
use soliddraft_assistant::mcp::{HttpCall, HttpServer, McpSession};
use soliddraft_assistant::tools::{Caller, ToolHost, ToolOutput, ToolSpec, schema, validate};
use soliddraft_document::DocumentId;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

const DRAWING_LIMIT: Duration = Duration::from_secs(15);
const DRAWING_TIMEOUT: &str =
    "Making the drawing exceeded its 15-second time limit. This request did not replace the drawing.";

/// The MCP endpoint of the running app.
pub struct McpEndpoint {
    /// The loopback server clients connect to.
    pub server: HttpServer,
    calls: Receiver<HttpCall>,
    session: McpSession,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    drawing: Option<DrawingJob>,
}

#[derive(PartialEq)]
struct Source {
    part: (DocumentId, DocumentId, u64),
    name: String,
    path: Option<PathBuf>,
    drawing: Option<(DocumentId, DocumentId, u64)>,
    setup: Option<NewDrawing>,
    locations: u64,
}

impl Source {
    fn of(state: &AppState) -> Self {
        Self {
            part: (state.doc.instance(), state.doc.model().id(), state.doc.revision()),
            name: state.doc.name.clone(),
            path: state.doc.path.clone(),
            drawing: state.drawing.session.as_ref().map(|s| (s.instance, s.doc.id, s.doc.edits)),
            setup: state.drawing.new.clone(),
            locations: state.locations_epoch,
        }
    }
}

struct DrawingJob {
    result: Receiver<Result<DrawingSession, String>>,
    source: Source,
    deadline: Instant,
    request: Option<Request>,
}

impl DrawingJob {
    #[cfg(not(target_arch = "wasm32"))]
    fn start(state: &AppState, new: NewDrawing, waker: Option<Arc<dyn Fn() + Send + Sync>>) -> Result<Self, String> {
        let source = Source::of(state);
        let mut doc = state.doc.fork();
        doc.path = state.doc.path.clone();
        let (tx, result) = std::sync::mpsc::channel();
        let deadline = Instant::now() + DRAWING_LIMIT;
        std::thread::Builder::new()
            .name("mcp-drawing".into())
            .spawn(move || {
                let made = crate::drawing::create_drawing_for_part(&doc, &new);
                let _ = tx.send(made);
                if let Some(wake) = waker {
                    wake();
                }
            })
            .map_err(|e| format!("Could not start the drawing: {e}"))?;
        Ok(Self { result, source, deadline, request: None })
    }

    #[cfg(target_arch = "wasm32")]
    fn start(_: &AppState, _: NewDrawing, _: Option<Arc<dyn Fn() + Send + Sync>>) -> Result<Self, String> {
        Err("Use the desktop app for HTTP MCP drawing requests.".into())
    }
}

impl McpEndpoint {
    pub(super) fn start(port: u16, waker: Option<Arc<dyn Fn() + Send + Sync>>) -> Result<Self, String> {
        let (server, calls) = HttpServer::start(port, waker.clone()).map_err(|e| format!("MCP server: {e}"))?;
        Ok(Self {
            server,
            calls,
            session: McpSession::new("soliddraft", env!("CARGO_PKG_VERSION"), crate::command_tools::MCP_INSTRUCTIONS),
            waker,
            drawing: None,
        })
    }

    pub(super) fn service(&mut self, state: &mut AppState) -> bool {
        self.poll_drawing(state);
        while let Ok(call) = self.calls.try_recv() {
            self.drive(Request::new(call), state);
        }
        self.drawing.as_ref().is_some_and(|job| job.request.is_some())
    }

    fn poll_drawing(&mut self, state: &mut AppState) {
        let Some(mut job) = self.drawing.take() else { return };
        let result = match job.result.try_recv() {
            Ok(result) => {
                if Instant::now() >= job.deadline {
                    Err(DRAWING_TIMEOUT.into())
                } else if job.source != Source::of(state) {
                    Err("The part or drawing changed while this drawing was being made. Try again.".into())
                } else {
                    result
                }
            }
            Err(TryRecvError::Disconnected) => Err("The drawing stopped before returning a result. Try again.".into()),
            Err(TryRecvError::Empty) => {
                if Instant::now() < job.deadline || job.request.is_none() {
                    self.drawing = Some(job);
                    return;
                }
                let request = job.request.take().expect("a request is waiting");
                // Keep the worker slot until it exits; retries cannot accumulate workers.
                self.drawing = Some(job);
                self.finish_drawing(request, Err(DRAWING_TIMEOUT.into()), state);
                return;
            }
        };
        if let Some(request) = job.request.take() {
            self.finish_drawing(request, result, state);
        }
    }

    fn finish_drawing(&mut self, mut request: Request, result: Result<DrawingSession, String>, state: &mut AppState) {
        let output = match result {
            Ok(session) => ToolOutput::json(&AppTools { state }.install_drawing(session)),
            Err(message) => ToolOutput::error(message),
        };
        let message = request.next().expect("the drawing call is waiting").clone();
        let reply = self.session.handle(&mut DrawingReply(output), message);
        request.answered(reply);
        self.drive(request, state);
    }

    fn drive(&mut self, mut request: Request, state: &mut AppState) {
        while let Some(message) = request.next().cloned() {
            let input = message["params"]
                .get("arguments")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let make = message.get("id").is_some()
                && message["method"] == "tools/call"
                && message["params"]["name"] == "make_drawing"
                && validate(&schema::object(&crate::app_tools::make_drawing_params()), &input).is_ok();
            let reply = if make {
                let started = if Instant::now() >= request.deadline {
                    Err(DRAWING_TIMEOUT.into())
                } else if self.drawing.is_some() {
                    Err("A previous drawing request is still finishing. Try again later.".into())
                } else {
                    AppTools { state }
                        .drawing_request(&input)
                        .and_then(|new| DrawingJob::start(state, new, self.waker.clone()))
                };
                match started {
                    Ok(mut job) => {
                        job.deadline = job.deadline.min(request.deadline);
                        job.request = Some(request);
                        self.drawing = Some(job);
                        return;
                    }
                    Err(message_text) => {
                        self.session.handle(&mut DrawingReply(ToolOutput::error(message_text)), message)
                    }
                }
            } else {
                self.session.handle(&mut AppTools { state }, message)
            };
            request.answered(reply);
        }
        request.finish();
    }
}

struct DrawingReply(ToolOutput);

impl ToolHost for DrawingReply {
    fn tools(&self) -> Vec<ToolSpec> {
        tool_specs(Surface::Mcp)
    }

    fn call(&mut self, _: &str, _: Value, _: &Caller) -> ToolOutput {
        self.0.clone()
    }
}

struct Batch {
    messages: VecDeque<Value>,
    replies: Vec<Value>,
}

struct Request {
    call: HttpCall,
    batches: Vec<Batch>,
    single: bool,
    deadline: Instant,
}

impl Request {
    fn new(call: HttpCall) -> Self {
        let (messages, single) = match &call.message {
            Value::Array(batch) if !batch.is_empty() => (batch.clone().into(), false),
            message => (VecDeque::from([message.clone()]), true),
        };
        Self {
            call,
            batches: vec![Batch { messages, replies: Vec::new() }],
            single,
            deadline: Instant::now() + DRAWING_LIMIT,
        }
    }

    fn next(&mut self) -> Option<&Value> {
        loop {
            let root = self.batches.len() == 1;
            let batch = self.batches.last_mut().expect("the root batch remains");
            match batch.messages.front() {
                Some(Value::Array(messages)) if !messages.is_empty() => {
                    let messages = messages.clone().into();
                    batch.messages.pop_front();
                    self.batches.push(Batch { messages, replies: Vec::new() });
                }
                Some(_) => return self.batches.last().unwrap().messages.front(),
                None if root => return None,
                None => {
                    let nested = self.batches.pop().unwrap();
                    if !nested.replies.is_empty() {
                        self.batches.last_mut().unwrap().replies.push(Value::Array(nested.replies));
                    }
                }
            }
        }
    }

    fn answered(&mut self, reply: Option<Value>) {
        let batch = self.batches.last_mut().unwrap();
        batch.messages.pop_front();
        if let Some(reply) = reply {
            batch.replies.push(reply);
        }
    }

    fn finish(mut self) {
        let replies = self.batches.pop().unwrap().replies;
        let reply = if self.single {
            replies.into_iter().next()
        } else {
            (!replies.is_empty()).then_some(Value::Array(replies))
        };
        self.call.respond(reply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use soliddraft_drawing::{DocKind, DocRef, Drawing};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::mpsc::channel;

    fn message(id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"make_drawing","arguments":{"template":"iso-a3"}}})
    }

    fn queued(endpoint: &McpEndpoint, message: Value) -> (Request, std::thread::JoinHandle<Value>) {
        let addr = endpoint.server.addr();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let body = message.to_string();
            write!(
                stream,
                "POST /mcp HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{response}");
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
        });
        let call = endpoint.calls.recv_timeout(Duration::from_secs(5)).unwrap();
        (Request::new(call), client)
    }

    fn blank_drawing(state: &AppState) -> DrawingSession {
        let model =
            DocRef::new(state.doc.model().id(), DocKind::Part, "Part1.dwprt", state.doc.model().semantic_hash());
        let template = soliddraft_drawing::library::template("iso-a3").template();
        DrawingSession::new(crate::drawing::DrawingDoc::new(Drawing::new(model, &template)))
    }

    fn job(state: &AppState, request: Request, result: Receiver<Result<DrawingSession, String>>) -> DrawingJob {
        DrawingJob {
            result,
            source: Source::of(state),
            deadline: Instant::now() + DRAWING_LIMIT,
            request: Some(request),
        }
    }

    #[test]
    fn a_timeout_replies_once_and_discards_a_late_success() {
        let mut endpoint = McpEndpoint::start(0, None).unwrap();
        let mut state = AppState::new(crate::new_document());
        let made = blank_drawing(&state);
        let (request, client) = queued(&endpoint, message(7));
        let (tx, rx) = channel();
        let mut pending = job(&state, request, rx);
        pending.deadline = Instant::now() - Duration::from_secs(1);
        endpoint.drawing = Some(pending);
        assert!(!endpoint.service(&mut state));
        let reply = client.join().unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("time limit"));
        assert!(state.drawing.session.is_none());

        let (retry, client) = queued(&endpoint, message(8));
        endpoint.drive(retry, &mut state);
        let reply = client.join().unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("previous drawing request"));
        assert!(endpoint.drawing.is_some());
        tx.send(Ok(made)).unwrap();
        endpoint.service(&mut state);
        assert!(endpoint.drawing.is_none());
        assert!(state.drawing.session.is_none());
    }

    #[test]
    fn a_completed_drawing_cannot_replace_changed_documents() {
        for replace_part in [false, true] {
            let mut endpoint = McpEndpoint::start(0, None).unwrap();
            let mut state = AppState::new(crate::new_document());
            state.drawing.session = Some(blank_drawing(&state));
            let drawing = state.drawing.session.as_ref().unwrap().doc.id;
            let made = blank_drawing(&state);
            let (request, client) = queued(&endpoint, message(3));
            let (tx, rx) = channel();
            endpoint.drawing = Some(job(&state, request, rx));
            if replace_part {
                state.doc = crate::new_document();
            } else {
                state.drawing.session.as_mut().unwrap().doc.touch();
            }
            let part = state.doc.model().id();
            tx.send(Ok(made)).unwrap();
            endpoint.service(&mut state);
            let reply = client.join().unwrap();
            assert_eq!(reply["result"]["isError"], true);
            assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("changed"));
            assert_eq!(state.doc.model().id(), part);
            assert_eq!(state.drawing.session.as_ref().unwrap().doc.id, drawing);
        }
    }

    #[test]
    fn a_completed_drawing_cannot_replace_reopened_documents() {
        for reopen_part in [false, true] {
            let mut endpoint = McpEndpoint::start(0, None).unwrap();
            let mut state = AppState::new(crate::new_document());
            state.drawing.session = Some(blank_drawing(&state));
            let drawing_id = state.drawing.session.as_ref().unwrap().doc.id;
            let made = blank_drawing(&state);
            let (request, client) = queued(&endpoint, message(3));
            let (tx, rx) = channel();
            endpoint.drawing = Some(job(&state, request, rx));
            if reopen_part {
                state.doc = crate::document::Document::from_model(state.doc.model().fork());
            } else {
                let mut reopened = blank_drawing(&state);
                reopened.doc.id = drawing_id;
                reopened.doc.drawing.meta.title = "Reopened drawing".into();
                state.drawing.session = Some(reopened);
            }
            tx.send(Ok(made)).unwrap();
            endpoint.service(&mut state);
            let reply = client.join().unwrap();
            assert_eq!(reply["result"]["isError"], true, "{reply}");
            assert_eq!(state.drawing.session.as_ref().unwrap().doc.id, drawing_id);
            if !reopen_part {
                assert_eq!(state.drawing.session.as_ref().unwrap().doc.drawing.meta.title, "Reopened drawing");
            }
        }
    }

    #[test]
    fn a_stopped_worker_returns_a_tool_refusal() {
        let mut endpoint = McpEndpoint::start(0, None).unwrap();
        let mut state = AppState::new(crate::new_document());
        let (request, client) = queued(&endpoint, message(4));
        let (tx, rx) = channel();
        endpoint.drawing = Some(job(&state, request, rx));
        drop(tx);
        assert!(!endpoint.service(&mut state));
        let reply = client.join().unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("stopped"));
    }

    #[test]
    fn another_query_can_finish_while_a_drawing_waits_and_its_refusal_is_preserved() {
        let mut endpoint = McpEndpoint::start(0, None).unwrap();
        let mut state = AppState::new(crate::new_document());
        let (request, drawing_client) = queued(&endpoint, message(1));
        let (tx, rx) = channel();
        endpoint.drawing = Some(job(&state, request, rx));
        let summary =
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"model_summary","arguments":{}}});
        let (request, query_client) = queued(&endpoint, summary);
        endpoint.drive(request, &mut state);
        assert_eq!(query_client.join().unwrap()["result"]["isError"], false);
        assert!(!drawing_client.is_finished());
        let reason = "kernel: mesh.precision-or-budget";
        tx.send(Err(reason.into())).unwrap();
        endpoint.service(&mut state);
        let reply = drawing_client.join().unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(reply["result"]["content"][0]["text"], reason);
    }

    #[test]
    fn deferred_batches_keep_reply_order_and_skip_notifications() {
        let mut endpoint = McpEndpoint::start(0, None).unwrap();
        let mut state = AppState::new(crate::new_document());
        let batch = json!([
            {"jsonrpc":"2.0","method":"ping"},
            message(1),
            [{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"model_summary","arguments":{}}}],
            {"jsonrpc":"2.0","id":3,"method":"ping"}
        ]);
        let (mut request, client) = queued(&endpoint, batch);
        let notification = request.next().unwrap().clone();
        request.answered(endpoint.session.handle(&mut AppTools { state: &mut state }, notification));
        let (tx, rx) = channel();
        endpoint.drawing = Some(job(&state, request, rx));
        tx.send(Ok(blank_drawing(&state))).unwrap();
        endpoint.service(&mut state);
        let reply = client.join().unwrap();
        assert_eq!(reply.as_array().unwrap().len(), 3);
        assert_eq!(reply[0]["id"], 1);
        assert_eq!(reply[0]["result"]["isError"], false);
        assert_eq!(reply[1][0]["id"], 2);
        assert_eq!(reply[2]["id"], 3);
        assert!(state.drawing.is_active());
    }

    #[test]
    fn drawings_in_a_batch_share_the_request_deadline() {
        let mut endpoint = McpEndpoint::start(0, None).unwrap();
        let mut state = AppState::new(crate::new_document());
        let (mut request, client) = queued(&endpoint, json!([message(1), message(2)]));
        request.deadline = Instant::now() - Duration::from_secs(1);
        endpoint.drive(request, &mut state);
        let reply = client.join().unwrap();
        assert_eq!(reply.as_array().unwrap().len(), 2);
        for result in reply.as_array().unwrap() {
            assert_eq!(result["result"]["isError"], true);
            assert!(result["result"]["content"][0]["text"].as_str().unwrap().contains("time limit"));
        }
        assert!(endpoint.drawing.is_none());
        assert!(state.drawing.session.is_none());
    }
}
