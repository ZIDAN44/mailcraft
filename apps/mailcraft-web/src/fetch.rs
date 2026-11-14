//! HTTP through the browser's `fetch`, for the JMAP transport and pictures.

use mailcraft_jmap::{Error, Request, Response, Result};
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;

use crate::js;

/// The browser's HTTP client.
#[derive(Clone, Copy, Default)]
pub struct Fetch;

/// A request as the JMAP transport and the event stream send it: CORS, and no cookies to other
/// sites (credentials go in the `Authorization` header only); `cache` when it's not the default.
fn request<'a>(
    method: &str,
    url: &str,
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
    body: &[u8],
    cache: Option<web_sys::RequestCache>,
) -> Result<web_sys::Request> {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_mode(web_sys::RequestMode::Cors);
    if let Some(cache) = cache {
        init.set_cache(cache);
    }
    let h = web_sys::Headers::new().map_err(|e| Error::Io(js(&e)))?;
    for (k, v) in headers {
        h.set(k, v).map_err(|e| Error::Protocol(js(&e)))?;
    }
    init.set_headers(&h);
    if !body.is_empty() {
        init.set_body(&js_sys::Uint8Array::from(body));
    }
    web_sys::Request::new_with_str_and_init(url, &init).map_err(|e| Error::Protocol(js(&e)))
}

impl mailcraft_jmap::Http for Fetch {
    async fn send(&self, req: Request) -> Result<Response> {
        let headers = req.headers.iter().map(|(k, v)| (k.as_str(), v.as_str()));
        let request = request(&req.method, &req.url, headers, &req.body, None)?;
        let window = web_sys::window().ok_or_else(|| Error::Io("no window".into()))?;
        let answer = JsFuture::from(window.fetch_with_request(&request)).await.map_err(|e| {
            // The browser says no more than this when the server doesn't allow this page (CORS).
            Error::Io(format!("{} (the server may not allow web pages to reach it)", js(&e)))
        })?;
        let resp: web_sys::Response = answer.dyn_into().map_err(|e| Error::Protocol(js(&e)))?;
        let status = resp.status();
        let too_big = || Error::Protocol(format!("the answer is larger than {} bytes", req.max));
        // Refused before reading when the server says how big it is; otherwise counted as it
        // comes, so a body past the cap is never held whole.
        let said = resp.headers().get("Content-Length").ok().flatten().and_then(|n| n.trim().parse::<u64>().ok());
        if said.is_some_and(|n| n > u64::try_from(req.max).unwrap_or(u64::MAX)) {
            if let Some(body) = resp.body() {
                let _ = body.cancel();
            }
            return Err(too_big());
        }
        let Some(stream) = resp.body() else {
            return Ok(Response { status, body: Vec::new() });
        };
        let reader = reader(&stream)?;
        let mut body = Vec::new();
        while let Some(chunk) = next_chunk(&reader).await? {
            let len = usize::try_from(chunk.length()).unwrap_or(usize::MAX);
            let start = body.len();
            if start.saturating_add(len) > req.max {
                let _ = reader.cancel();
                return Err(too_big());
            }
            body.resize(start + len, 0);
            if let Some(dst) = body.get_mut(start..) {
                chunk.copy_to(dst);
            }
        }
        Ok(Response { status, body })
    }
}

fn reader(stream: &web_sys::ReadableStream) -> Result<web_sys::ReadableStreamDefaultReader> {
    stream.get_reader().dyn_into().map_err(|e| Error::Protocol(js(&e)))
}

/// The next piece of a body, or `None` at its end.
async fn next_chunk(reader: &web_sys::ReadableStreamDefaultReader) -> Result<Option<js_sys::Uint8Array>> {
    let chunk = JsFuture::from(reader.read()).await.map_err(|e| Error::Io(js(&e)))?;
    let field = |name: &str| js_sys::Reflect::get(&chunk, &wasm_bindgen::JsValue::from_str(name)).unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
    if field("done").as_bool().unwrap_or(true) {
        return Ok(None);
    }
    field("value").dyn_into::<js_sys::Uint8Array>().map(Some).map_err(|e| Error::Protocol(js(&e)))
}

/// Read an event stream (a GET with the `Authorization` header), handing each event to `each`
/// (first an `open` one, once the server has answered) until it says stop (`false`) or the
/// stream ends.
pub async fn events(url: &str, auth: &str, each: &mut dyn FnMut(mailcraft_jmap::sse::Sse) -> bool) -> Result<()> {
    let headers = [("Authorization", auth), ("Accept", "text/event-stream")];
    // Not a `Cache-Control` header: servers that allow pages often don't allow that one.
    let request = request("GET", url, headers, &[], Some(web_sys::RequestCache::NoStore))?;
    let window = web_sys::window().ok_or_else(|| Error::Io("no window".into()))?;
    let resp: web_sys::Response =
        JsFuture::from(window.fetch_with_request(&request)).await.map_err(|e| Error::Io(js(&e)))?.dyn_into().map_err(|e| Error::Protocol(js(&e)))?;
    if resp.status() != 200 {
        return Err(Error::Protocol(format!("the event stream answered HTTP {}", resp.status())));
    }
    let body = resp.body().ok_or_else(|| Error::Protocol("the event stream has no body".into()))?;
    let reader = reader(&body)?;
    let mut sse = mailcraft_jmap::sse::SseReader::default();
    if !each(mailcraft_jmap::sse::Sse { event: "open".into(), data: String::new() }) {
        let _ = reader.cancel();
        return Ok(());
    }
    loop {
        let Some(chunk) = next_chunk(&reader).await? else {
            return Err(Error::Io("the event stream ended".into()));
        };
        for e in sse.push(&chunk.to_vec()).map_err(Error::Protocol)? {
            if !each(e) {
                let _ = reader.cancel();
                return Ok(());
            }
        }
    }
}

/// Wait `ms` milliseconds.
pub async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}
